pub mod setup;

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex}, task::Poll,
};

use anyhow::anyhow;
use futures_util::AsyncReadExt;
use num_traits::FromPrimitive;
use pool::mt_datatypes::PoolVec;
use s2n_quic::{
    application, client::{Connect, ConnectionAttempt}, connection::{BidirectionalStreamAcceptor, Handle, ReceiveStreamAcceptor}, provider::datagram::default::Receiver, stream::{BidirectionalStream, ReceiveStream, SendStream} 
};
use spki::der::Decode;
use tokio::io::AsyncWriteExt;

use super::{
    connection::ConnectionStats,
    errors::{BanType, Banned, ConnectionErrorCode},
    event::{NetworkEventConnectingClosed, NetworkEventConnectingFailed, NetworkEventDisconnect},
    network::Network,
    network_async::NetworkAsync,
    networks::Networks,
    traits::{
        NetworkConnectingInterface, NetworkConnectionInterface, NetworkEndpointInterface,
        NetworkIncomingInterface, UnreliableUnorderedError, NUM_BIDI_STREAMS,
    },
    types::{
        NetworkClientInitOptions, NetworkInOrderChannel, NetworkServerCertMode,
        NetworkServerCertModeResult, NetworkServerInitOptions,
    },
};

#[derive(Default)]
pub struct S2nNetworkConnectingWrapperChannel {
    in_order_packets: VecDeque<PoolVec<u8>>,
    open_bi: Option<(SendStream, ReceiveStream)>,
}

#[derive(Clone)]
pub struct S2nNetworkConnectionWrapper {
    handle: Handle,bi_acceptor: Arc<Mutex< BidirectionalStreamAcceptor>>,uni_acceptor:Arc<Mutex< ReceiveStreamAcceptor>>,
    channels: Arc<
        std::sync::Mutex<
            HashMap<
                NetworkInOrderChannel,
                Arc<tokio::sync::Mutex<S2nNetworkConnectingWrapperChannel>>,
            >,
        >,
    >,
    stream_window: usize,
}

impl S2nNetworkConnectionWrapper {
    async fn write_bytes_chunked(
        send_stream: &mut SendStream,
        packet: PoolVec<u8>,
    ) -> anyhow::Result<()> {
        let packet_len = packet.len() as u64;
        let send_buffer = [packet_len.to_le_bytes().to_vec(), packet.take()].concat();
        let written_bytes = send_stream.write_all(send_buffer.as_slice()).await;
        if let Err(err) = written_bytes {
            Err(anyhow!(format!("packet write failed: {}", err.to_string())))
        } else {
            match send_stream.flush().await {
                Ok(_) => Ok(()),
                Err(err) => Err(anyhow!(format!("packet flush failed: {}", err.to_string()))),
            }
        }
    }
}

#[async_trait::async_trait]
impl NetworkConnectionInterface for S2nNetworkConnectionWrapper {
    async fn send_unreliable_unordered(
        &self,
        data: PoolVec<u8>,
    ) -> anyhow::Result<(), (PoolVec<u8>, UnreliableUnorderedError)> {
        let pack_bytes = bytes::Bytes::copy_from_slice(&data[..]);
        let res = self
            .handle
            .datagram_mut(
                |sender: &mut s2n_quic::provider::datagram::default::Sender| match sender
                    .send_datagram(pack_bytes)
                {
                    Ok(_) => Ok(()),
                    Err(err) => match err {
                        s2n_quic::provider::datagram::default::DatagramError::QueueAtCapacity {..} => Err((data, UnreliableUnorderedError::TooLarge)),
                        s2n_quic::provider::datagram::default::DatagramError::ExceedsPeerTransportLimits {..} => Err((data, UnreliableUnorderedError::TooLarge)),
                        s2n_quic::provider::datagram::default::DatagramError::ConnectionError { error, .. } => Err((data, UnreliableUnorderedError::ConnectionClosed(error.into()))),
                        _ => Err((data, UnreliableUnorderedError::ConnectionClosed(anyhow!("Unimplemented error!")))),
                                            
                    },
                },
            )
            .map_err(|err| match err {
                s2n_quic::provider::event::query::Error::ConnectionLockPoisoned => {
                    (PoolVec::new_without_pool(), UnreliableUnorderedError::ConnectionClosed(err.into()))
                }
                s2n_quic::provider::event::query::Error::ContextTypeMismatch => {
                    (PoolVec::new_without_pool(), UnreliableUnorderedError::ConnectionClosed(err.into()))
                }
                _ => (PoolVec::new_without_pool(), UnreliableUnorderedError::ConnectionClosed(err.into())),
            })?;
        /*.send_datagram(pack_bytes)
        .map_err(|err| match err {
            SendDatagramError::Disabled | SendDatagramError::UnsupportedByPeer => {
                (data, UnreliableUnorderedError::Disabled)
            }
            SendDatagramError::ConnectionLost(err) => {
                (data, UnreliableUnorderedError::ConnectionClosed(err.into()))
            }
            SendDatagramError::TooLarge => (data, UnreliableUnorderedError::TooLarge),
        })?;*/
        res
    }

    async fn read_unreliable_unordered(&self) -> anyhow::Result<Vec<u8>> {        
        let res = futures_util::future::poll_fn(|cx| {
            match self.handle.datagram_mut(|recv: &mut Receiver| recv.poll_recv_datagram(cx)) {
                Ok(poll_value) => poll_value.map(Ok),
                Err(query_err) => Poll::Ready(Err(query_err)),
            }
        })
        .await?;
        match res {
            Ok(res) => Ok(res.to_vec()),
            Err(err) => Err(anyhow!(err.to_string())),
        }
    }

    async fn send_unordered_reliable(&self, data: PoolVec<u8>) -> anyhow::Result<()> {
        let uni =  futures_util::future::poll_fn(|cx| {
            self.handle.clone().poll_open_send_stream(cx)
       }).await;
        if let Ok(mut stream) = uni {
            let written_bytes = stream.write_all(data.as_slice()).await;
            if let Err(_written_bytes) = written_bytes {
                Err(anyhow!("packet write failed."))
            } else {
                let finish_res = stream.finish();
                if let Err(err) = finish_res {
                    Err(anyhow!(format!(
                        "packet finish failed: {}",
                        err.to_string()
                    )))
                } else {
                    Ok(())
                }
            }
        } else {
            Err(anyhow!(format!(
                "sent stream err: {}",
                uni.unwrap_err().to_string()
            )))
        }
    }

    async fn read_unordered_reliable<
        F: FnOnce(anyhow::Result<Vec<u8>>) -> tokio::task::JoinHandle<()> + Send + 'static,
    >(
        &self,
        on_data: F,
    ) -> anyhow::Result<()> {
        let uni = futures_util::future::poll_fn(|cx| {
             self.uni_acceptor.lock().unwrap().poll_accept_receive_stream(cx) 
        })
        .await?;
        let stream_window = self.stream_window;
        match uni {
            Some(mut recv_stream) => {
                tokio::spawn(async move {
                    let mut pkt = Vec::default();
                    match recv_stream.read_to_end(&mut pkt).await {
                        Ok(read_res) => {
                            // ignore error
                            let _ = on_data(Ok(pkt)).await;
                        }
                        Err(read_err) => {
                            on_data(Err(anyhow!(format!(
                                "connection stream acception failed {}",
                                read_err
                            ))))
                            .await?;
                        }
                    }
                    anyhow::Ok(())
                });
                anyhow::Ok(())
            }
            None => Err(anyhow!(format!(
                "connection was closed",
            ))),
        }
    }

    async fn push_ordered_reliable_packet_in_order(
        &self,
        data: PoolVec<u8>,
        channel: NetworkInOrderChannel,
    ) {
        let cur_channel = {
            let mut channels = self.channels.lock().unwrap();
            let has_global = channels.contains_key(&NetworkInOrderChannel::Global);
            let reserved_channels = if has_global { 0 } else { 1 };
            let channel = if channels.len() >= NUM_BIDI_STREAMS as usize - reserved_channels {
                // always fall back to the global channel if limit is reached
                NetworkInOrderChannel::Global
            } else {
                channel
            };
            channels.entry(channel).or_default();
            let cur_channel = channels.get_mut(&channel).unwrap().clone();
            drop(channels);
            cur_channel
        };
        cur_channel.lock().await.in_order_packets.push_back(data);
    }

    async fn send_one_ordered_reliable(
        &self,
        channel: NetworkInOrderChannel,
    ) -> anyhow::Result<()> {
        let cur_channel = {
            let mut channels = self.channels.lock().unwrap();
            let cur_channel = channels
                .get_mut(&channel)
                .cloned()
                .or_else(|| channels.get_mut(&NetworkInOrderChannel::Global).cloned());
            cur_channel
        };
        if let Some(cur_channel) = cur_channel {
            let mut cur_channel = cur_channel.lock().await;
            let packet_res = cur_channel.in_order_packets.pop_front();
            if let Some(packet) = packet_res {
                if let Some((send_stream, _)) = cur_channel.open_bi.as_mut() {
                    Self::write_bytes_chunked(send_stream, packet).await
                } else {
                    match futures_util::future::poll_fn(|cx| {
                        self.handle.clone().poll_open_bidirectional_stream(cx)
                   }).await.map(|s| s.split()) {
                        Ok(  ( recv, send)) => {
                            cur_channel.open_bi = Some((send, recv));
                            Self::write_bytes_chunked(
                                &mut cur_channel.open_bi.as_mut().unwrap().0,
                                packet,
                            )
                            .await
                        }
                        Err(err) => Err(anyhow!(err.to_string())),
                    }
                }
            } else {
                Err(anyhow!("No packet was queued."))
            }
        } else {
            Err(anyhow!("Channel did not exist."))
        }
    }

    async fn read_ordered_reliable<
        F: Fn(anyhow::Result<Vec<u8>>) -> tokio::task::JoinHandle<()> + Send + Sync + 'static,
    >(
        &self,
        on_data: F,
    ) -> anyhow::Result<()> {
        let stream_window = self.stream_window;
        match futures_util::future::poll_fn(|cx| {
            self.bi_acceptor.lock().unwrap().poll_accept_bidirectional_stream(cx)
       }).await?.map(|s| s.split()) {
            Some(( mut recv_stream, _)) => {
                tokio::spawn(async move {
                    let mut len_buff: [u8; std::mem::size_of::<u64>()] = Default::default();
                    'read_loop: loop {
                        match recv_stream.read_exact(&mut len_buff).await {
                            Ok(_) => {
                                let read_buff_len = u64::from_le_bytes(len_buff);
                                if read_buff_len > stream_window as u64 {
                                    on_data(Err(anyhow!("read size exceeded max length."))).await?;
                                    break 'read_loop;
                                } else {
                                    let mut read_buff: Vec<u8> = Vec::new();
                                    read_buff.resize(read_buff_len as usize, Default::default());

                                    match recv_stream.read_exact(read_buff.as_mut()).await {
                                        Ok(_) => {
                                            on_data(Ok(read_buff)).await?;
                                        }
                                        Err(err) => {
                                            on_data(Err(anyhow!(err.to_string()))).await?;
                                            break 'read_loop;
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                on_data(Err(anyhow!(err.to_string()))).await?;
                                break 'read_loop;
                            }
                        }
                    }

                    anyhow::Ok(())
                });
                Ok(())
            }
            None => {
                return Err(anyhow!("Connection was closed"));
            }
        }
    }

    async fn close(&self, error_code: ConnectionErrorCode, reason: &str) {
        self.handle
            .close(application::Error::new(error_code as u64).unwrap());
            }

    fn close_reason(&self) -> Option<NetworkEventDisconnect> {
        None
        /*self.handle.close_reason().map(|err| match err {
            ConnectionError::VersionMismatch
            | ConnectionError::CidsExhausted
            | ConnectionError::TransportError(_) => NetworkEventDisconnect::Other(err.to_string()),
            ConnectionError::ConnectionClosed(connection_close) => {
                NetworkEventDisconnect::ConnectionClosed(NetworkEventConnectingClosed::Other(
                    connection_close.to_string(),
                ))
            }
            ConnectionError::ApplicationClosed(application_close) => {
                let reason = String::from_utf8_lossy(&application_close.reason).into();
                NetworkEventDisconnect::ConnectionClosed(
                    match ConnectionErrorCode::from_u64(application_close.error_code.into_inner()) {
                        Some(code) => match code {
                            ConnectionErrorCode::Kicked => {
                                NetworkEventConnectingClosed::Kicked(reason)
                            }
                            ConnectionErrorCode::Banned => NetworkEventConnectingClosed::Banned(
                                serde_json::from_str(&reason).unwrap_or_else(|_| Banned {
                                    msg: BanType::Custom("unknown reason".to_string()),
                                    until: None,
                                }),
                            ),
                            ConnectionErrorCode::Shutdown => {
                                NetworkEventConnectingClosed::Shutdown(reason)
                            }
                        },
                        None => NetworkEventConnectingClosed::Other(reason),
                    },
                )
            }
            ConnectionError::Reset => {
                NetworkEventDisconnect::ConnectionClosed(NetworkEventConnectingClosed::Reset)
            }
            ConnectionError::TimedOut => NetworkEventDisconnect::TimedOut,
            ConnectionError::LocallyClosed => NetworkEventDisconnect::LocallyClosed,
        })*/
    }

    fn remote_addr(&self) -> SocketAddr {
        self.handle.remote_addr().unwrap()
    }

    fn peer_identity(&self) -> x509_cert::Certificate {
        let certs = self.handle.peer_identity().unwrap();
        let certs: &Vec<rustls::pki_types::CertificateDer> = certs.downcast_ref().unwrap();
        x509_cert::Certificate::from_der(&certs[0]).unwrap()
    }

    fn stats(&self) -> ConnectionStats {
        let mut stats = self.handle.stats();

        stats.path.rtt = self.con.rtt();

        ConnectionStats {
            ping: stats.path.rtt,
            packets_lost: stats.path.lost_packets,
            packets_sent: stats.path.sent_packets,
            bytes_sent: stats.udp_tx.bytes,
            bytes_recv: stats.udp_rx.bytes,
        }
    }
}

pub struct S2nNetworkConnectingWrapper {
    connecting: ConnectionAttempt,
    stream_window: usize,
}

impl Future for S2nNetworkConnectingWrapper {
    type Output = Result<S2nNetworkConnectionWrapper, NetworkEventConnectingFailed>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let con = Pin::new(&mut self.connecting).poll(cx);
        con.map(|f| match f {
            Ok(connection) => {
                let (handle, acceptor) = connection.split();
                let (bi, uni) = acceptor.split();
                Ok(S2nNetworkConnectionWrapper {
                handle,bi_acceptor: Arc::new(Mutex::new(bi)),uni_acceptor: Arc::new(Mutex::new(uni)),

                channels: Default::default(),
                stream_window: self.stream_window,
            })},
            Err(err) => Err(match err {
                ConnectionError::VersionMismatch
                | ConnectionError::CidsExhausted
                | ConnectionError::TransportError(_) => {
                    NetworkEventConnectingFailed::Other(err.to_string())
                }
                ConnectionError::ConnectionClosed(connection_close) => {
                    NetworkEventConnectingFailed::ConnectionClosed(
                        NetworkEventConnectingClosed::Other(connection_close.to_string()),
                    )
                }
                ConnectionError::ApplicationClosed(application_close) => {
                    let reason = String::from_utf8_lossy(&application_close.reason).into();
                    NetworkEventConnectingFailed::ConnectionClosed(
                        match ConnectionErrorCode::from_u64(
                            application_close.error_code.into_inner(),
                        ) {
                            Some(code) => match code {
                                ConnectionErrorCode::Kicked => {
                                    NetworkEventConnectingClosed::Kicked(reason)
                                }
                                ConnectionErrorCode::Banned => {
                                    NetworkEventConnectingClosed::Banned(
                                        serde_json::from_str(&reason).unwrap_or_else(|_| Banned {
                                            msg: BanType::Custom("unknown reason".to_string()),
                                            until: None,
                                        }),
                                    )
                                }
                                ConnectionErrorCode::Shutdown => {
                                    NetworkEventConnectingClosed::Shutdown(reason)
                                }
                            },
                            None => NetworkEventConnectingClosed::Other(reason),
                        },
                    )
                }
                ConnectionError::Reset => NetworkEventConnectingFailed::ConnectionClosed(
                    NetworkEventConnectingClosed::Reset,
                ),
                ConnectionError::TimedOut => NetworkEventConnectingFailed::TimedOut,
                ConnectionError::LocallyClosed => NetworkEventConnectingFailed::LocallyClosed,
            }),
        })
    }
}

impl NetworkConnectingInterface<S2nNetworkConnectionWrapper> for S2nNetworkConnectingWrapper {
    fn remote_addr(&self) -> SocketAddr {
        self.connecting.remote_address()
    }
}

pub struct S2nNetworkIncomingWrapper {
    inc: quinn::Incoming,
    stream_window: usize,
}

impl NetworkIncomingInterface<S2nNetworkConnectingWrapper> for S2nNetworkIncomingWrapper {
    fn remote_addr(&self) -> SocketAddr {
        self.inc.remote_address()
    }

    fn accept(self) -> anyhow::Result<S2nNetworkConnectingWrapper> {
        Ok(S2nNetworkConnectingWrapper {
            connecting: self.inc.accept()?,
            stream_window: self.stream_window,
        })
    }
}

#[derive(Clone)]
enum Endpoint {
    Client(s2n_quic::Client),
    Server(Arc<s2n_quic::Server>),
}

#[derive(Clone)]
pub struct S2nEndpointWrapper {
    endpoint: Endpoint,
    must_retry_inc: bool,
    stream_window: usize,
}

#[async_trait::async_trait]
impl NetworkEndpointInterface<S2nNetworkConnectingWrapper, S2nNetworkIncomingWrapper>
    for S2nEndpointWrapper
{
    fn connect(
        &self,
        addr: std::net::SocketAddr,
        server_name: &str,
    ) -> anyhow::Result<S2nNetworkConnectingWrapper, NetworkEventConnectingFailed> {
        let Endpoint::Client(endpoint) = &self.endpoint else {
            return Err(NetworkEventConnectingFailed::Other(
                "Connect can only be called from a client endpoint".to_string(),
            ));
        };
        let res = endpoint.connect(Connect::new(addr).with_server_name(server_name));
        Ok(S2nNetworkConnectingWrapper {
            connecting: res,
            stream_window: self.stream_window,
        })
    }

    fn close(&self, error_code: ConnectionErrorCode, reason: &str) {
        self.endpoint
            .close((error_code as u32).into(), reason.as_bytes());
    }

    fn make_server_endpoint(
        bind_addr: std::net::SocketAddr,
        cert_mode: NetworkServerCertMode,
        options: &NetworkServerInitOptions,
    ) -> anyhow::Result<(Self, NetworkServerCertModeResult)> {
        let (endpoint, cert) = make_server_endpoint(bind_addr, cert_mode, options)?;
        Ok((
            Self {
                endpoint,
                must_retry_inc: !options.disable_retry_on_connect,
                stream_window: options
                    .base
                    .stream_receive_window
                    .unwrap_or(1024u32 * 64u32) as usize,
            },
            cert,
        ))
    }

    fn make_client_endpoint(
        bind_addr: std::net::SocketAddr,
        options: &NetworkClientInitOptions,
    ) -> anyhow::Result<Self> {
        let res = make_client_endpoint(bind_addr, options)?;
        Ok(Self {
            endpoint: res,
            must_retry_inc: false,
            stream_window: options
                .base
                .stream_receive_window
                .unwrap_or(1024u32 * 1024u32) as usize,
        })
    }

    async fn accept(&self) -> Option<S2nNetworkIncomingWrapper> {
        while let Some(inc) = self.endpoint.accept().await {
            if self.must_retry_inc && !inc.remote_address_validated() {
                inc.retry().unwrap();
            } else {
                return Some(S2nNetworkIncomingWrapper {
                    inc,
                    stream_window: self.stream_window,
                });
            }
        }
        None
    }

    fn sock_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }
}

pub type S2nNetworks = Networks<
    S2nEndpointWrapper,
    S2nNetworkConnectionWrapper,
    S2nNetworkConnectingWrapper,
    S2nNetworkIncomingWrapper,
>;

pub type S2nNetwork = Network<
    S2nEndpointWrapper,
    S2nNetworkConnectionWrapper,
    S2nNetworkConnectingWrapper,
    S2nNetworkIncomingWrapper,
    0,
>;

pub type S2nNetworkAsync = NetworkAsync<
    S2nEndpointWrapper,
    S2nNetworkConnectionWrapper,
    S2nNetworkConnectingWrapper,
    S2nNetworkIncomingWrapper,
    0,
>;
