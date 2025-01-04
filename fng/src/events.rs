use api_macros::events_mod;

#[events_mod("../")]
pub mod events {
    use crate::fng_tiles::FngSpikeTiles;

    #[derive(Debug, Hiarc, Clone, Copy, Serialize, Deserialize)]
    pub enum CharacterEventMod {
        DespawnBySpike {
            id: CharacterId,
            killer_id: Option<CharacterId>,
            spike: FngSpikeTiles,
        },
    }
}
