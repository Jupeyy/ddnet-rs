pub mod core;
pub mod player;
pub mod pos {
    pub use ::vanilla::entities::character::pos::*;
}
pub mod hook {
    pub use ::vanilla::entities::character::hook::*;
}
pub mod score {
    pub use ::vanilla::entities::character::score::*;
}

use api_macros::character_mod;

#[character_mod("../")]
pub mod character {
    use crate::{events::events::CharacterEventMod, fng_tiles::FngSpikeTiles};

    impl CharacterPhaseDead {
        fn push_character_event(
            _simulation_events: &SimulationWorldEvents,
            _id: CharacterId,
            _killer_id: Option<CharacterId>,
            _weapon: GameWorldActionKillWeapon,
        ) {
        }
    }

    impl CharacterCore {
        fn get_core_mut_and_input(
            &mut self,
            reusable_core: &CharacterReusableCore,
        ) -> (&mut Core, &CharacterInput) {
            if reusable_core.debuffs.contains_key(&CharacterDebuff::Freeze) {
                self.modifications.dummy_frozen_input = self.input;
                self.modifications.dummy_frozen_input.state.dir.set(0);
                self.modifications.dummy_frozen_input.state.hook.set(false);
                self.modifications.dummy_frozen_input.state.fire.set(false);
                self.modifications.dummy_frozen_input.state.jump.set(false);
                (&mut self.core, &self.modifications.dummy_frozen_input)
            } else {
                (&mut self.core, &self.input)
            }
        }
    }

    #[derive(Debug, Hiarc, Default, Serialize, Deserialize, Copy, Clone)]
    pub struct CharacterCoreMod {
        pub frozen_by: Option<CharacterId>,
        pub dummy_frozen_input: CharacterInput,
    }

    impl Character {
        fn respawn_weapons(reusable_core: &mut CharacterReusableCore) {
            let wpn = Weapon {
                cur_ammo: None,
                next_ammo_regeneration_tick: 0.into(),
            };
            reusable_core.weapons.clear();
            reusable_core.weapons.insert(WeaponType::Hammer, wpn);
            reusable_core.weapons.insert(WeaponType::Laser, wpn);
        }

        #[needs_super]
        pub fn handle_input_change(
            &mut self,
            pipe: &mut SimulationPipeCharacter,
            diff: CharacterInputConsumableDiff,
        ) -> EntityTickResult {
            if self
                .reusable_core
                .debuffs
                .contains_key(&CharacterDebuff::Freeze)
            {
                self.handle_weapon_switch(diff.weapon_diff, diff.weapon_req);
                EntityTickResult::None
            } else {
                self._super_handle_input_change(pipe, diff)
            }
        }

        fn default_active_weapon() -> WeaponType {
            WeaponType::Laser
        }

        fn handle_game_layer_tiles(&mut self, tile: &Tile) -> CharacterDamageResult {
            if tile.index == DdraceTileNum::Death as u8 {
                self.die(None, GameWorldActionKillWeapon::World, Default::default());
                CharacterDamageResult::Death
            } else {
                let spike = if tile.index == FngSpikeTiles::Golden as u8 {
                    Some(FngSpikeTiles::Golden)
                } else if tile.index == FngSpikeTiles::Normal as u8 {
                    Some(FngSpikeTiles::Normal)
                } else if tile.index == FngSpikeTiles::Red as u8 {
                    Some(FngSpikeTiles::Red)
                } else if tile.index == FngSpikeTiles::Blue as u8 {
                    Some(FngSpikeTiles::Blue)
                } else if tile.index == FngSpikeTiles::Green as u8 {
                    Some(FngSpikeTiles::Green)
                } else if tile.index == FngSpikeTiles::Purple as u8 {
                    Some(FngSpikeTiles::Purple)
                } else {
                    None
                };
                if let Some(spike) = spike {
                    self.die(
                        self.core.modifications.frozen_by,
                        GameWorldActionKillWeapon::Ninja,
                        Default::default(),
                    );
                    self.simulation_events
                        .push_world(SimulationEventWorldEntityType::Character {
                            ev: CharacterEvent::Mod(CharacterEventMod::DespawnBySpike {
                                id: self.base.game_element_id,
                                killer_id: self.core.modifications.frozen_by,
                                spike,
                            }),
                        });
                    CharacterDamageResult::Death
                } else {
                    CharacterDamageResult::None
                }
            }
        }

        pub fn take_damage_from(
            self_char: &mut Character,
            _self_char_id: &CharacterId,
            killer_id: CharacterId,
            force: &vec2,
            _source: &vec2,
            friendly_fire_ty: FriendlyFireTy,
            _dmg_amount: u32,
            _from: DamageTypes,
            by: DamageBy,
        ) -> CharacterDamageResult {
            self_char.core.core.vel += vec2::new(force.x * 320.0 * 0.01, force.y * 120.0 * 0.01);
            if matches!(
                (friendly_fire_ty, by),
                (
                    FriendlyFireTy::Dmg,
                    DamageBy::Weapon {
                        weapon: WeaponType::Laser,
                        ..
                    }
                ),
            ) {
                if !self_char
                    .reusable_core
                    .debuffs
                    .contains_key(&CharacterDebuff::Freeze)
                {
                    self_char.reusable_core.debuffs.insert(
                        CharacterDebuff::Freeze,
                        BuffProps {
                            remaining_tick: (TICKS_PER_SECOND * 10).into(),
                            interact_tick: Default::default(),
                            interact_cursor_dir: Default::default(),
                            interact_val: 0.0,
                        },
                    );
                    self_char.core.modifications.frozen_by = Some(killer_id);
                    CharacterDamageResult::Damage
                } else {
                    CharacterDamageResult::None
                }
            } else {
                CharacterDamageResult::None
            }
        }
    }
}
