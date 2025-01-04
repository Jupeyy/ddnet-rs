use api_macros::match_manager_mod;

#[match_manager_mod("../")]
pub mod match_manager {
    use game_interface::types::render::character::CharacterDebuff;

    use crate::{entities::character::character::BuffProps, fng_tiles::FngSpikeTiles};

    impl MatchManager {
        fn mod_event(
            world: &mut GameWorld,
            game_match: &mut Match,
            game_options: &GameOptions,
            ev: &CharacterEventMod,
        ) {
            match ev {
                CharacterEventMod::DespawnBySpike {
                    killer_id, spike, ..
                } => {
                    if let Some(char) =
                        killer_id.and_then(|killer_id| world.characters.get_mut(&killer_id))
                    {
                        let team_point = match spike {
                            FngSpikeTiles::Golden => {
                                char.score.set(char.score.get() + 7);
                                10
                            }
                            FngSpikeTiles::Normal => {
                                char.score.set(char.score.get() + 3);
                                5
                            }
                            FngSpikeTiles::Red | FngSpikeTiles::Blue => {
                                let check_side = if matches!(spike, FngSpikeTiles::Red) {
                                    MatchSide::Red
                                } else {
                                    MatchSide::Blue
                                };
                                match char.core.side {
                                    Some(side) => {
                                        if side == check_side {
                                            char.score.set(char.score.get() + 5);
                                        } else {
                                            char.score.set(char.score.get() - 5);

                                            char.reusable_core.debuffs.insert(
                                                CharacterDebuff::Freeze,
                                                BuffProps {
                                                    remaining_tick: (TICKS_PER_SECOND * 5).into(),
                                                    interact_tick: Default::default(),
                                                    interact_cursor_dir: Default::default(),
                                                    interact_val: Default::default(),
                                                },
                                            );
                                        }
                                    }
                                    None => {
                                        char.score.set(char.score.get() + 5);
                                    }
                                }
                                10
                            }
                            FngSpikeTiles::Green => {
                                char.score.set(char.score.get() + 6);
                                10
                            }
                            FngSpikeTiles::Purple => {
                                char.score.set(char.score.get() + 5);
                                10
                            }
                        };
                        if let (MatchType::Sided { scores }, Some(team)) =
                            (&mut game_match.ty, char.core.side)
                        {
                            scores[team as usize] += team_point;
                        }
                        game_match.win_check(game_options, &world.scores, false);
                    }
                }
            }
        }
    }
}
