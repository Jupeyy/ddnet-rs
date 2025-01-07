use std::{
    cell::RefCell,
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    ops::Range,
    rc::Rc,
};

use anyhow::anyhow;
use binds::binds::{
    bind_to_str, gen_local_player_action_hash_map, gen_local_player_action_hash_map_rev,
    syn_to_bind, syn_to_bind_keys, BindActionsLocalPlayer, BindKey,
};
use client_types::console::{
    entries_to_parser, ConsoleEntry, ConsoleEntryCmd, ConsoleEntryVariable,
};
use client_ui::console::utils::{syn_vec_to_config_val, try_apply_config_val};
use command_parser::parser::{
    self, format_args, CommandArg, CommandArgType, CommandType, ParserCache, Syn,
};
use config::{
    config::ConfigEngine,
    parsing::parse_conf_values_as_str_list,
    traits::{ConfigFromStrOperation, ConfigInterface},
};
use egui::Color32;
use game_config::config::ConfigGame;
use hiarc::{hiarc_safer_rc_refcell, Hiarc};
use ui_base::ui::UiCreator;

use super::console::ConsoleRender;

#[derive(Debug, Hiarc)]
pub enum LocalConsoleEvent {
    Connect {
        addresses: Vec<SocketAddr>,
        can_start_local_server: bool,
    },
    /// A bind command was executed
    Bind {
        // The bind was added to the player's profile
        was_player_profile: bool,
    },
    /// A unbind command was executed
    Unbind {
        // The bind was added to the player's profile
        was_player_profile: bool,
    },
    /// Switch to an dummy or the main player
    ChangeDummy {
        dummy_index: Option<usize>,
    },
    /// Switch to the next dummy or the main player
    ToggleDummy,
    ConfigVariable {
        name: String,
    },
    LocalPlayerAction(BindActionsLocalPlayer),
    Quit,
}

#[hiarc_safer_rc_refcell]
#[derive(Debug, Default, Hiarc)]
pub struct LocalConsoleEvents {
    events: Vec<LocalConsoleEvent>,
}

#[hiarc_safer_rc_refcell]
impl LocalConsoleEvents {
    pub fn push(&mut self, ev: LocalConsoleEvent) {
        self.events.push(ev)
    }
}

#[hiarc_safer_rc_refcell]
impl super::console::ConsoleEvents<LocalConsoleEvent> for LocalConsoleEvents {
    #[hiarc_trait_is_immutable_self]
    fn take(&mut self) -> Vec<LocalConsoleEvent> {
        std::mem::take(&mut self.events)
    }
    #[hiarc_trait_is_immutable_self]
    fn push(&mut self, ev: LocalConsoleEvent) {
        self.events.push(ev);
    }
}

pub type LocalConsole = ConsoleRender<LocalConsoleEvent, ()>;

#[derive(Debug, Default)]
pub struct LocalConsoleBuilder {}

impl LocalConsoleBuilder {
    fn register_commands(console_events: LocalConsoleEvents, list: &mut Vec<ConsoleEntry>) {
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "push".into(),
            usage: "push <var>".into(),
            description: "Push a new item to a config variable of type array.".into(),
            cmd: Rc::new(|config_engine, config_game, path| {
                let path = syn_vec_to_config_val(path).unwrap_or_default();
                if config_engine
                    .try_set_from_str(path.clone(), None, None, None, ConfigFromStrOperation::Push)
                    .is_err()
                    && config_game
                        .try_set_from_str(
                            path.clone(),
                            None,
                            None,
                            None,
                            ConfigFromStrOperation::Push,
                        )
                        .is_err()
                {
                    return Err(anyhow::anyhow!("No array variable with that name found"));
                }
                Ok(format!("Added new entry for {path}"))
            }),
            args: vec![CommandArg {
                ty: CommandArgType::CommandIdent,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "pop".into(),
            usage: "pop <var>".into(),
            description: "Pop the last item of a config variable of type array.".into(),
            cmd: Rc::new(|config_engine, config_game, path| {
                let path = syn_vec_to_config_val(path).unwrap_or_default();
                if config_engine
                    .try_set_from_str(path.clone(), None, None, None, ConfigFromStrOperation::Pop)
                    .is_err()
                    && config_game
                        .try_set_from_str(
                            path.clone(),
                            None,
                            None,
                            None,
                            ConfigFromStrOperation::Pop,
                        )
                        .is_err()
                {
                    return Err(anyhow::anyhow!("No array variable with that name found"));
                }
                Ok(format!("Removed last entry from {path}"))
            }),
            args: vec![CommandArg {
                ty: CommandArgType::CommandIdent,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "rem".into(),
            usage: "rem <var>[key]".into(),
            description: "Remove an item from a config variable of type object.".into(),
            cmd: Rc::new(|config_engine, config_game, path| {
                let path = syn_vec_to_config_val(path).unwrap_or_default();
                if config_engine
                    .try_set_from_str(path.clone(), None, None, None, ConfigFromStrOperation::Rem)
                    .is_err()
                    && config_game
                        .try_set_from_str(
                            path.clone(),
                            None,
                            None,
                            None,
                            ConfigFromStrOperation::Rem,
                        )
                        .is_err()
                {
                    return Err(anyhow::anyhow!("No record variable with that key found"));
                }
                Ok(format!("Removed entry {path}"))
            }),
            args: vec![CommandArg {
                ty: CommandArgType::CommandIdent,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "reset".into(),
            usage: "reset <var>".into(),
            description: "Reset the value of a config variable to its default.".into(),
            cmd: Rc::new(|config_engine, config_game, path| {
                let path = syn_vec_to_config_val(path).unwrap_or_default();
                if path.is_empty() {
                    return Err(anyhow::anyhow!("You cannot reset the whole config at once"));
                }
                let res_engine = config_engine.try_set_from_str(
                    path.clone(),
                    None,
                    None,
                    None,
                    ConfigFromStrOperation::Reset,
                );
                let res_game = config_game.try_set_from_str(
                    path.clone(),
                    None,
                    None,
                    None,
                    ConfigFromStrOperation::Reset,
                );
                match (res_engine, res_game) {
                    (Ok(val), _) => Ok(format!("Reset value for {path} to: {val}")),
                    (_, Ok(val)) => Ok(format!("Reset value for {path} to: {val}")),
                    (Err(err1), Err(err2)) => Err(anyhow::anyhow!(
                        "No variable with that key found: {err1}. {err2}"
                    )),
                }
            }),
            args: vec![CommandArg {
                ty: CommandArgType::CommandIdent,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));

        fn toggle(
            config_engine: &mut ConfigEngine,
            config_game: &mut ConfigGame,
            path: &[(Syn, Range<usize>)],
        ) -> anyhow::Result<String> {
            let Syn::Command(cmd) = &path[0].0 else {
                return Err(anyhow!(
                    "Argument must be a command, but was: {:?}",
                    path[0].0
                ));
            };
            anyhow::ensure!(
                cmd.args.len() >= 2,
                "The given command must take at least 1 argument for toggle to make sense."
            );

            let res_engine = config_engine.try_set_from_str(
                cmd.cmd_text.clone(),
                None,
                None,
                None,
                ConfigFromStrOperation::Set,
            );
            let res_game = config_game.try_set_from_str(
                cmd.cmd_text.clone(),
                None,
                None,
                None,
                ConfigFromStrOperation::Set,
            );

            match res_engine.or(res_game) {
                Ok(val) => {
                    let arg1 = format_args(&cmd.args[0..cmd.args.len() / 2]);
                    let arg2 = format_args(&cmd.args[cmd.args.len() / 2..]);

                    let new_val = if arg1 == val { arg2 } else { arg1 };

                    try_apply_config_val(
                        &cmd.cmd_text,
                        &[(Syn::Text(new_val), 0..0)],
                        config_engine,
                        config_game,
                    )
                    .map_err(|err| anyhow!(err))
                    .map(|new_val| {
                        format!(
                            "Toggled value for {} from {} to {}",
                            cmd.cmd_text, val, new_val
                        )
                    })
                }
                Err(err) => Err(err.into()),
            }
        }
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "toggle".into(),
            usage: "toggle <var> <arg> <arg>".into(),
            description: "Toggle a config variable between two args.".into(),
            cmd: Rc::new(|config_engine, config_game, path| {
                toggle(config_engine, config_game, path)
            }),
            args: vec![CommandArg {
                ty: CommandArgType::CommandDoubleArg,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "+toggle".into(),
            usage: "+toggle <var> <arg> <arg>".into(),
            description:
                "Toggle a config variable between two args until the pressed key is released again."
                    .into(),
            cmd: Rc::new(|config_engine, config_game, path| {
                toggle(config_engine, config_game, path)
            }),
            args: vec![CommandArg {
                ty: CommandArgType::CommandDoubleArg,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));

        let actions_map = gen_local_player_action_hash_map();
        let actions_map_rev = gen_local_player_action_hash_map_rev();

        for (name, &action) in actions_map.iter() {
            let events = console_events.clone();
            list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
                name: name.to_string(),
                usage: format!("triggers a player action: {}", name),
                description: format!("Triggers the player action: {}", name),
                cmd: Rc::new(move |_config_engine, _config_game, _path| {
                    events.push(LocalConsoleEvent::LocalPlayerAction(action));
                    Ok(String::default())
                }),
                args: vec![],
                allows_partial_cmds: false,
            }));
        }

        let keys_arg = CommandArg {
            ty: CommandArgType::TextArrayFrom {
                from: {
                    let mut res = vec![];
                    for i in 'a'..='z' {
                        res.push(i.to_string());
                    }
                    for i in '0'..='9' {
                        res.push(i.to_string());
                    }
                    for i in 0..35 {
                        res.push(format!("f{}", i + 1));
                    }

                    for i in 0..=9 {
                        res.push(format!("numpad{}", i));
                    }
                    res.push("numpad_subtract".to_string());
                    res.push("numpad_add".to_string());
                    res.push("numpad_multiply".to_string());
                    res.push("numpad_divide".to_string());

                    for i in 0..=9 {
                        res.push(format!("digit{}", i));
                    }

                    res.push("page_up".to_string());
                    res.push("page_down".to_string());

                    res.push("enter".to_string());
                    res.push("escape".to_string());

                    res.push("pause".to_string());

                    res.push("left".to_string());
                    res.push("right".to_string());
                    res.push("middle".to_string());

                    res.push("arrow_left".to_string());
                    res.push("arrow_right".to_string());
                    res.push("arrow_up".to_string());
                    res.push("arrow_down".to_string());

                    res.push("control_left".to_string());
                    res.push("control_right".to_string());

                    res.push("shift_left".to_string());
                    res.push("shift_right".to_string());

                    res.push("alt_left".to_string());
                    res.push("alt_right".to_string());

                    res.push("space".to_string());
                    res.push("tab".to_string());

                    res.push("wheel_down".to_string());
                    res.push("wheel_up".to_string());
                    // TODO: add lot more
                    res.into_iter().map(|s| s.try_into().unwrap()).collect()
                },
                separator: '+',
            },
            user_ty: None,
        };

        fn str_to_bind_keys_lossy(
            keys_arg: &CommandArg,
            cache: &RefCell<ParserCache>,
            bind: &str,
        ) -> Vec<Vec<BindKey>> {
            let cmds = parser::parse(
                bind,
                &entries_to_parser(&[ConsoleEntry::Cmd(ConsoleEntryCmd {
                    name: "bind".to_string(),
                    usage: "dummy".to_string(),
                    description: "dummy".to_string(),
                    cmd: Rc::new(|_, _, _| Ok("".into())),
                    args: vec![keys_arg.clone()],
                    allows_partial_cmds: false,
                })]),
                &mut cache.borrow_mut(),
            );

            let mut res: Vec<_> = Default::default();
            for cmd in &cmds {
                match cmd {
                    CommandType::Full(cmd) => match syn_to_bind_keys(&mut cmd.args.iter()) {
                        Ok(keys) => {
                            res.push(keys);
                        }
                        Err(err) => {
                            log::info!(
                                "ignored invalid bind (syntax error): \
                                    {bind}, err: {err}"
                            );
                        }
                    },
                    CommandType::Partial(err) => {
                        log::info!("ignored invalid bind: {bind}, err: {err}");
                    }
                }
            }
            res
        }
        fn unbind(
            player_index: usize,
            is_dummy: bool,
            config_game: &mut ConfigGame,
            path: &[(Syn, Range<usize>)],
            keys_arg: &CommandArg,
            cache: &RefCell<ParserCache>,
            events: &LocalConsoleEvents,
        ) -> anyhow::Result<Vec<String>> {
            let mut keys = syn_to_bind_keys(&mut path.iter())?;
            keys.sort();
            let player = config_game
                .players
                .get_mut(player_index)
                .ok_or_else(|| anyhow!("player index is out of bounds {player_index}"))?;
            let mut res = Vec::new();
            player.binds.retain(|bind| {
                let binds = str_to_bind_keys_lossy(
                    keys_arg,
                    cache,
                    &bind
                        .split_whitespace()
                        .take(2)
                        .collect::<Vec<&str>>()
                        .join(" "),
                );
                let keep = binds.into_iter().all(|mut bind_keys| {
                    bind_keys.sort();
                    bind_keys != keys
                });

                if !keep {
                    res.push(bind.clone());
                }
                keep
            });
            if !res.is_empty() {
                events.push(LocalConsoleEvent::Unbind {
                    was_player_profile: !is_dummy,
                });
            }
            Ok(res)
        }
        fn unbindes_to_str(unbinds: Vec<String>) -> String {
            if unbinds.is_empty() {
                "Nothing was unbound, this key bind does not exist.".to_string()
            } else {
                format!("Unbound following binds:\n{}", unbinds.join("\n"))
            }
        }
        #[allow(clippy::too_many_arguments)]
        fn bind(
            player_index: usize,
            is_dummy: bool,
            config_game: &mut ConfigGame,
            path: &[(Syn, Range<usize>)],
            keys_arg: &CommandArg,
            cache: &RefCell<ParserCache>,
            actions_map: &HashMap<&'static str, BindActionsLocalPlayer>,
            actions_map_rev: &HashMap<BindActionsLocalPlayer, &'static str>,
            events: &LocalConsoleEvents,
        ) -> anyhow::Result<String> {
            let (keys, action) = syn_to_bind(path, actions_map)?;
            let unbound = unbind(
                player_index,
                is_dummy,
                config_game,
                path,
                keys_arg,
                cache,
                events,
            )?;
            let new_bind = bind_to_str(&keys, action, actions_map_rev);
            let mut res = format!("Added new bind: {new_bind}.");
            if !unbound.is_empty() {
                res.push_str(&format!(
                    "\nReplacing existing bind(s):\n{}",
                    unbound.join("\n")
                ))
            }

            config_game
                .players
                .get_mut(player_index)
                .ok_or_else(|| anyhow!("player index was out of bounds: {player_index}"))?
                .binds
                .push(new_bind);

            events.push(LocalConsoleEvent::Bind {
                was_player_profile: !is_dummy,
            });
            Ok(res)
        }
        // bind for player
        let events = console_events.clone();
        let cache_shared = Rc::new(RefCell::new(ParserCache::default()));
        let cache = cache_shared.clone();
        let keys_arg_cmd = keys_arg.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "bind".into(),
            usage: "bind <keys> <commands>".into(),
            description: "Binds commands to a single key or key chain.".into(),
            cmd: Rc::new(move |_config_engine, config_game, path| {
                bind(
                    config_game.profiles.main as usize,
                    false,
                    config_game,
                    path,
                    &keys_arg_cmd,
                    &cache,
                    &actions_map,
                    &actions_map_rev,
                    &events,
                )
            }),
            args: vec![
                keys_arg.clone(),
                CommandArg {
                    ty: CommandArgType::Commands,
                    user_ty: None,
                },
            ],
            allows_partial_cmds: false,
        }));
        // bind for dummy
        let actions_map = gen_local_player_action_hash_map();
        let actions_map_rev = gen_local_player_action_hash_map_rev();
        let events = console_events.clone();
        let cache = cache_shared.clone();
        let keys_arg_cmd = keys_arg.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "bind_dummy".into(),
            usage: "bind_dummy <keys> <commands>".into(),
            description: "Binds commands to a single key or key chain for the dummy profile."
                .into(),
            cmd: Rc::new(move |_config_engine, config_game, path| {
                bind(
                    config_game.profiles.dummy.index as usize,
                    true,
                    config_game,
                    path,
                    &keys_arg_cmd,
                    &cache,
                    &actions_map,
                    &actions_map_rev,
                    &events,
                )
            }),
            args: vec![
                keys_arg.clone(),
                CommandArg {
                    ty: CommandArgType::Commands,
                    user_ty: None,
                },
            ],
            allows_partial_cmds: false,
        }));

        let keys_arg_cmd = keys_arg.clone();
        // unbind for player
        let cache = cache_shared.clone();
        let events = console_events.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "unbind".into(),
            usage: "unbind <keys>".into(),
            description: "Unbinds commands from a single key or key chain.".into(),
            cmd: Rc::new(move |_config_engine, config_game, path| {
                unbind(
                    config_game.profiles.main as usize,
                    false,
                    config_game,
                    path,
                    &keys_arg_cmd,
                    &cache,
                    &events,
                )
                .map(unbindes_to_str)
            }),
            args: vec![keys_arg.clone()],
            allows_partial_cmds: false,
        }));
        let keys_arg_cmd = keys_arg.clone();
        let cache = cache_shared.clone();
        let events = console_events.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "unbind_dummy".into(),
            usage: "unbind_dummy <keys>".into(),
            description: "Unbinds commands from a single key or key chain for the dummy profile."
                .into(),
            cmd: Rc::new(move |_config_engine, config_game, path| {
                unbind(
                    config_game.profiles.dummy.index as usize,
                    true,
                    config_game,
                    path,
                    &keys_arg_cmd,
                    &cache,
                    &events,
                )
                .map(unbindes_to_str)
            }),
            args: vec![keys_arg],
            allows_partial_cmds: false,
        }));

        let console_events_cmd = console_events.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "connect".into(),
            usage: "connect <ip:port>".into(),
            description: "Connects to a server of the given ip & port.".into(),
            cmd: Rc::new(move |_, _, path| {
                let (Syn::Text(text), _) = path
                    .first()
                    .ok_or_else(|| anyhow!("expected ip & port, but found nothing"))?
                else {
                    return Err(anyhow!("Expected a text that represents the ip+port"));
                };
                let text = if !text.contains(":") {
                    format!("{text}:8303")
                } else {
                    text.clone()
                };
                let addresses = text.to_socket_addrs()?.collect();
                console_events_cmd.push(LocalConsoleEvent::Connect {
                    addresses,
                    can_start_local_server: true,
                });
                Ok(format!("Trying to connect to {text}"))
            }),
            args: vec![CommandArg {
                ty: CommandArgType::Text,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));

        let console_events_cmd = console_events.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "change_dummy".into(),
            usage: "change_dummy <index>".into(),
            description: "Switches to a dummy, or the main player (index 0).".into(),
            cmd: Rc::new(move |_, _, path| {
                let (Syn::Number(index), _) = path
                    .first()
                    .ok_or_else(|| anyhow!("expected an index, but found nothing"))?
                else {
                    return Err(anyhow!("Expected an index"));
                };
                let index: usize = index.parse()?;
                console_events_cmd.push(LocalConsoleEvent::ChangeDummy {
                    dummy_index: if index == 0 {
                        None
                    } else {
                        Some(index.saturating_sub(1))
                    },
                });
                Ok("".to_string())
            }),
            args: vec![CommandArg {
                ty: CommandArgType::Number,
                user_ty: None,
            }],
            allows_partial_cmds: false,
        }));

        let console_events_cmd = console_events.clone();
        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "toggle_dummy".into(),
            usage: "toggle_dummy".into(),
            description: "Toggles between a dummy and the main player.".into(),
            cmd: Rc::new(move |_, _, _| {
                console_events_cmd.push(LocalConsoleEvent::ToggleDummy);
                Ok("".to_string())
            }),
            args: Default::default(),
            allows_partial_cmds: false,
        }));

        list.push(ConsoleEntry::Cmd(ConsoleEntryCmd {
            name: "quit".into(),
            usage: "quit the client".into(),
            description: "Closes the client.".into(),
            cmd: Rc::new(move |_, _, _| {
                console_events.push(LocalConsoleEvent::Quit);
                Ok("Bye bye".to_string())
            }),
            args: vec![],
            allows_partial_cmds: false,
        }));
    }

    pub fn build(creator: &UiCreator) -> LocalConsole {
        let console_events: LocalConsoleEvents = Default::default();
        let mut entries: Vec<ConsoleEntry> = Vec::new();
        let val = ConfigEngine::conf_value();
        let events_var = console_events.clone();
        let var_on_set = Rc::new(move |name: &str| {
            events_var.push(LocalConsoleEvent::ConfigVariable {
                name: name.to_string(),
            });
        });
        parse_conf_values_as_str_list(
            "".to_string(),
            &mut |entry, _| {
                entries.push(ConsoleEntry::Var(ConsoleEntryVariable {
                    full_name: entry.name,
                    usage: entry.usage,
                    description: entry.description,
                    args: entry.args,
                    on_set: var_on_set.clone(),
                }));
            },
            val,
            "".into(),
            Default::default(),
        );
        let val = ConfigGame::conf_value();
        parse_conf_values_as_str_list(
            "".to_string(),
            &mut |entry, _| {
                entries.push(ConsoleEntry::Var(ConsoleEntryVariable {
                    full_name: entry.name,
                    usage: entry.usage,
                    description: entry.description,
                    args: entry.args,
                    on_set: var_on_set.clone(),
                }));
            },
            val,
            "".into(),
            Default::default(),
        );
        Self::register_commands(console_events.clone(), &mut entries);
        ConsoleRender::new(
            creator,
            entries,
            Box::new(console_events),
            Color32::from_rgba_unmultiplied(0, 0, 0, 150),
            (),
        )
    }
}
