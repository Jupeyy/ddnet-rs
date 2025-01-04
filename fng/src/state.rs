use api_macros::state_mod;

#[state_mod("../")]
pub mod state {
    impl GameState {
        fn get_game_type_from_conf(conf: ConfigGameType) -> GameType {
            match conf {
                ConfigGameType::Ctf => GameType::Team,
                ConfigGameType::Dm => GameType::Solo,
                ConfigGameType::Fng => GameType::Team,
            }
        }

        fn get_mod_name_from_conf(
            conf: ConfigGameType,
        ) -> NetworkString<MAX_PHYSICS_GAME_TYPE_NAME_LEN> {
            match conf {
                ConfigGameType::Dm => "dm".try_into().unwrap(),
                ConfigGameType::Ctf => "ctf".try_into().unwrap(),
                ConfigGameType::Fng => "fng".try_into().unwrap(),
            }
        }
    }
}
