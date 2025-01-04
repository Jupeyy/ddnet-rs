use api_macros::config_mod;

#[config_mod("../")]
pub mod config {
    #[derive(
        Debug,
        Default,
        Clone,
        Copy,
        Serialize,
        Deserialize,
        ConfigInterface,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
    )]
    pub enum ConfigGameType {
        #[default]
        Dm,
        Ctf,
        Fng,
    }
}
