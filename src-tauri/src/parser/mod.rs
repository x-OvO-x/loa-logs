use std::{cmp::max, collections::{HashMap, HashSet}, time::Duration, thread};

use chrono::{DateTime, Utc};
use lazy_static::lazy_static;

pub(crate) mod models;
use models::*;
mod log_lines;
use log_lines::*;
use tauri::{Window, Wry};

lazy_static! {
    static ref NPC_DATA: HashMap<i32, Npc> = {
        let json_str = include_str!("../../meter-data/Npc.json");
        serde_json::from_str(json_str).unwrap()
    };
    static ref SKILL_DATA: HashMap<i32, SkillData> = {
        let json_str = include_str!("../../meter-data/Skill.json");
        serde_json::from_str(json_str).unwrap()
    };
    static ref SKILL_EFFECT_DATA: HashMap<i32, SkillEffectData> = {
        let json_str = include_str!("../../meter-data/SkillEffect.json");
        serde_json::from_str(json_str).unwrap()
    };
    static ref SKILL_BUFF_DATA: HashMap<i32, SkillBuffData> = {
        let json_str = include_str!("../../meter-data/SkillBuff.json");
        serde_json::from_str(json_str).unwrap()
    };
}

pub fn parse_log(lines: Vec<String>) -> Result<Vec<Encounter>, String> {
    let encounters: Vec<Encounter> = Vec::new();
    let mut encounters = Some(encounters);
    let mut encounter: Encounter = Default::default();
    for line in lines {
        parse_line(None, &mut encounters, &mut false, &mut encounter, line);
    }
    
    let mut encounters = encounters.unwrap().clone();

    for mut encounter in encounters.iter_mut() {
        encounter.duration = encounter.last_combat_packet - encounter.fight_start;
        let duration_seconds = encounter.duration as f64 / 1000 as f64;
        encounter.encounter_damage_stats.dps = (encounter.encounter_damage_stats.total_damage_dealt as f64 / duration_seconds) as i64;
        let most_damage_taken_entity = encounter.entities
            .values()
            .max_by_key(|entity| entity.damage_stats.damage_taken)
            .unwrap();
        encounter.encounter_damage_stats.most_damage_taken_entity = MostDamageTakenEntity {
            name: most_damage_taken_entity.name.clone(),
            damage_taken: most_damage_taken_entity.damage_stats.damage_taken,
        };

        let mut to_remove: Vec<String> = Vec::new();
        for (key, mut entity) in encounter.entities.iter_mut() {
            if entity.max_hp <= 0 {
                to_remove.push(key.clone());
                continue;
            }
            
            entity.damage_stats.dps = (entity.damage_stats.damage_taken as f64 / duration_seconds) as i64;
            for (_, mut skill) in entity.skills.iter_mut() {
                skill.dps = (skill.total_damage as f64 / duration_seconds) as i64;
            }
        }

        for key in to_remove {
            encounter.entities.remove(&key);
        }
    }


    Ok(encounters)
}

pub fn parse_line(window: Option<&Window<Wry>>, encounters: &mut Option<Vec<Encounter>>, reset: &mut bool, encounter: &mut Encounter, line: String) {
    println!("{}", line);
    if line.is_empty() {
        return;
    }
    
    let line_split: Vec<&str> = line.trim().split('|').collect();
    if line_split.len() < 2 || line_split[0].is_empty() {
        return;
    }

    let log_type: i32;
    match line_split[0].parse::<i32>() {
        Ok(t) => log_type = t,
        Err(_) => {
            println!("Could not parse log type");
            return;
        }
    }

    let timestamp: i64;
    match line_split[1].parse::<DateTime<Utc>>() {
        Ok(t) => timestamp = t.timestamp_millis(),
        Err(_) => {
            println!("Could not parse timestamp");
            return
        }
    }

    // if there is no id associated with the log line, we can ignore it. i think.
    if line_split[2] == "0" {
        return;
    }

    match log_type {
        0 => on_message(encounter, timestamp, &line_split),
        1 => on_init_env(window, encounters, encounter, timestamp, &line_split),
        2 => on_phase_transition(window, encounters, reset, encounter, &line_split),
        3 => on_new_pc(encounter, timestamp, &line_split),
        4 => on_new_npc(encounter, timestamp, &line_split),
        5 => on_death(encounter, timestamp, &line_split),
        6 => on_skill_start(encounter, timestamp, &line_split),
        7 => on_skill_stage(encounter, &line_split),
        8 => on_damage(reset, encounter, timestamp, &line_split),
        9 => on_heal(encounter, &line_split),
        10 => on_buff(encounter, &line_split),
        12 => on_counterattack(encounter, &line_split),
        _ => {}
    }
}

fn reset(encounter: &mut Encounter, clone: &Encounter) {
    encounter.fight_start = 0;
    encounter.entities = HashMap::new();
    encounter.current_boss_name = "".to_string();
    encounter.encounter_damage_stats = Default::default();
    encounter.reset = false;
    if !clone.local_player.is_empty() {
        if let Some(player) = clone.entities.get(&clone.local_player) {
            encounter.local_player = clone.local_player.to_string();
            encounter.entities.insert(clone.local_player.to_string(), Entity {
                id: player.id.to_string(),
                name: player.name.to_string(),
                class: player.class.to_string(),
                class_id: player.class_id,
                entity_type: EntityType::PLAYER,
                gear_score: player.gear_score,
                last_update: Utc::now().timestamp_millis(),
                ..Default::default()
            });
        }
    }
}

fn soft_reset(encounter: &mut Encounter) {
    let clone = encounter.clone();
    reset(encounter, &clone);
    encounter.current_boss_name = clone.current_boss_name.clone();
    for (key, entity) in clone.entities {
        encounter.entities.insert(key, Entity {
            last_update: Utc::now().timestamp_millis(),
            name: entity.name,
            id: entity.id,
            npc_id: entity.npc_id,
            class: entity.class,
            class_id: entity.class_id,
            entity_type: entity.entity_type,
            gear_score: entity.gear_score,
            max_hp: entity.max_hp,
            current_hp: entity.current_hp,
            is_dead: entity.is_dead,
            ..Default::default()
        });
    }
}

fn split_encounter(encounters: &mut Option<Vec<Encounter>>, encounter: &mut Encounter, is_soft_reset: bool) {
    if encounter.fight_start != 0 && 
        (encounter.encounter_damage_stats.total_damage_dealt != 0 || encounter.encounter_damage_stats.total_damage_taken != 0) {
            if encounters.is_some() {
                encounters.as_mut().unwrap().push(encounter.clone());
            }
    }
    if is_soft_reset {
        soft_reset(encounter);
    } else {
        reset(encounter, &encounter.clone());
    }
}

fn on_message(_encounter: &mut Encounter, _timestamp: i64, line: &[&str]) {
    println!("Message: {:?}", line);
}

fn on_init_env(window: Option<&Window<Wry>>, encounters: &mut Option<Vec<Encounter>>, encounter: &mut Encounter, timestamp: i64, line: &[&str]) {
    let init_env = LogInitEnv { 
        player_id: line[2]
    };

    if init_env.player_id.is_empty() {
        return;
    }

    if let Some(player) = encounter.entities.get_mut(&encounter.local_player) {
        player.id = init_env.player_id.to_string();
        player.last_update = timestamp;
    } else {
        encounter.local_player = String::from("You");
        encounter.entities.insert(String::from("You"), Entity {
            id: init_env.player_id.to_string(),
            name: String::from("You"),
            entity_type: EntityType::PLAYER,
            last_update: timestamp,
            ..Default::default()
        });
    }
    // is live
    if encounters.is_none() {
        encounter.entities.retain(|_, v| v.name == encounter.local_player || v.damage_stats.damage_dealt > 0);
        encounter.current_boss_name = "".to_string();
        thread::sleep(Duration::from_millis(6000));
        soft_reset(encounter);
    } else {
        split_encounter(encounters, encounter, false)
    }

    if window.is_some() {
        window.unwrap().emit("zone-change", "")
            .expect("failed to emit zone-change");
    }
}

fn on_phase_transition(window: Option<&Window<Wry>>, encounters: &mut Option<Vec<Encounter>>, reset: &mut bool, encounter: &mut Encounter, line: &[&str]) {
    let phase_transition = LogPhaseTransition { 
        raid_result: match line[2].parse::<i32>().unwrap() {
            0 => RaidResult::RAID_RESULT,
            1 => RaidResult::GUARDIAN_DEAD,
            2 => RaidResult::RAID_END,
            _ => RaidResult::UNKNOWN,
        }
    };

    if window.is_some() {
        window.unwrap().emit("phase-transition", phase_transition.raid_result)
            .expect("failed to emit phase-transition");
    }

    if encounters.is_none() {
        *reset = true;
        encounter.reset = true;
    } else {
        split_encounter(encounters, encounter, true)
    }
}

fn on_new_pc(encounter: &mut Encounter, timestamp: i64, line: &[&str]) {
    let mut gear_score = match line[7].parse::<f64>() {
        Ok(score) => score,
        Err(_) => 0.0
    };

    if gear_score > 1655.0 || gear_score < 0.0 {
        gear_score = 0.0;
    }

    let new_pc = LogNewPc {
        id: line[2],
        name: if line[3].is_empty() { "Unknown Entity" } else { line[3] },
        class_id: line[4].parse::<i32>().unwrap_or_default(),
        class: if line[5].is_empty() { "Unknown Class" } else { line[5] },
        level: line[6].parse::<i32>().unwrap_or_default(),
        gear_score,
        current_hp: line[8].parse::<i64>().unwrap_or_default(),
        max_hp: line[9].parse::<i64>().unwrap_or_default(),
        entity_type: EntityType::PLAYER
    };
    
    if !encounter.local_player.is_empty() {
        if let Some(player) = encounter.entities.get_mut(&encounter.local_player) {
            if new_pc.id == player.id {
                encounter.local_player = new_pc.name.to_string();
            }
        }
    }

    if let Some(player) = encounter.entities.get_mut(new_pc.name) {
        player.id = new_pc.id.to_string();
        player.class_id = new_pc.class_id;
        player.class = new_pc.class.to_string();
        player.gear_score = new_pc.gear_score;
        player.current_hp = new_pc.current_hp;
        player.max_hp = new_pc.max_hp;
        player.last_update = timestamp;
    } else {
        encounter.entities.retain(|_, entity| entity.id != new_pc.id);
        encounter.entities.insert(new_pc.name.to_string(), Entity {
            id: new_pc.id.to_string(),
            name: new_pc.name.to_string(),
            class_id: new_pc.class_id,
            class: new_pc.class.to_string(),
            gear_score: new_pc.gear_score,
            current_hp: new_pc.current_hp,
            max_hp: new_pc.max_hp,
            entity_type: EntityType::PLAYER,
            last_update: timestamp,
            ..Default::default()
        });
    }
}

fn on_new_npc(encounter: &mut Encounter, timestamp: i64, line: &[&str]) {
    let new_npc = LogNewNpc {
        id: line[2],
        npc_id: line[3].parse::<i32>().unwrap_or_default(),
        name: if line[4].is_empty() { "Unknown Entity" } else { line[4] },
        current_hp: line[5].parse::<i64>().unwrap_or_default(),
        max_hp: line[6].parse::<i64>().unwrap_or_default(),
        entity_type: EntityType::UNKNOWN,
    };

    if let Some(npc) = encounter.entities.get_mut(new_npc.name) {
        npc.id = new_npc.id.to_string();
        npc.npc_id = new_npc.npc_id;
        npc.name = new_npc.name.to_string();
        npc.current_hp = new_npc.current_hp;
        npc.max_hp = new_npc.max_hp;
        npc.last_update = timestamp;
        if let Some((_, npc_info)) = NPC_DATA.get_key_value(&new_npc.npc_id) {
            if npc_info.grade == "boss" || npc_info.grade == "raid" || npc_info.grade == "epic_raid" || npc_info.grade == "commander" {
                npc.entity_type = EntityType::BOSS;
            } else {
                npc.entity_type = EntityType::NPC;
            }
        }
    } else {
        let mut entity_type = EntityType::NPC;
        if let Some((_, npc_info)) = NPC_DATA.get_key_value(&new_npc.npc_id) {
            if npc_info.grade == "boss" || npc_info.grade == "raid" || npc_info.grade == "epic_raid" || npc_info.grade == "commander" {
                entity_type = EntityType::BOSS;
            }
        }
        encounter.entities.insert(new_npc.name.to_string(), Entity {
            id: new_npc.id.to_string(),
            npc_id: new_npc.npc_id,
            name: new_npc.name.to_string(),
            current_hp: new_npc.current_hp,
            max_hp: new_npc.max_hp,
            entity_type,
            last_update: timestamp,
            ..Default::default()
        });
    }
    
    if encounter.current_boss_name.is_empty() {
        if let Some((_, npc)) = NPC_DATA.get_key_value(&new_npc.npc_id) {
            if npc.grade == "boss" || npc.grade == "raid" || npc.grade == "epic_raid" || npc.grade == "commander" {
                encounter.current_boss_name = new_npc.name.to_string();
            }
        }
    } else if !encounter.current_boss_name.is_empty() {
        // if for some reason current_boss_name is not in the entities list, reset it
        if let Some(boss) = encounter.entities.get(&encounter.current_boss_name.to_string()) {
            if new_npc.max_hp > boss.max_hp {
                if let Some((_, npc)) = NPC_DATA.get_key_value(&new_npc.npc_id) {
                    if npc.grade == "boss" || npc.grade == "raid" || npc.grade == "epic_raid" || npc.grade == "commander" {
                        encounter.current_boss_name = new_npc.name.to_string();
                    }
                }
            }
        } else {
            encounter.current_boss_name = "".to_string();
        }
    }
}

fn on_death(encounter: &mut Encounter, timestamp: i64, line: &[&str]) {
    let new_death = LogDeath {
        id: line[2],
        name: if line[3].is_empty() { "Unknown Entity" } else { line[3] },
        killer_id: line[4],
        killer_name: if line[5].is_empty() { "Unknown Entity" } else { line[5] }
    };

    if let Some(entity) = encounter.entities.get_mut(new_death.name) {
        // the entity that died has the same name as another entity, but with different id?
        if entity.id != new_death.id {
            return;
        }
        let deaths: i64;
        if entity.is_dead { deaths = entity.damage_stats.deaths } else { deaths = 1 }
        entity.is_dead = true;
        entity.damage_stats.deaths = deaths;
        entity.damage_stats.death_time = timestamp;
        entity.last_update = timestamp;
    } else {
        encounter.entities.insert(new_death.name.to_string(), Entity {
            id: new_death.id.to_string(),
            name: new_death.name.to_string(),
            is_dead: true,
            damage_stats: DamageStats {
                deaths: 1,
                death_time: timestamp,
                ..Default::default()
            },
            last_update: timestamp,
            ..Default::default()
        });
    }
}

fn on_skill_start(encounter: &mut Encounter, timestamp: i64, line: &[&str]) {
    let skill_start = LogSkillStart {
        id: line[2],
        name: if line[3].is_empty() { "Unknown Entity" } else { line[3] },
        skill_id: line[4].parse::<i32>().unwrap_or_default(),
        skill_name: if line[5].is_empty() { "Unknown Skill" } else { line[5] },
    };

    let mut entity = encounter.entities.entry(skill_start.name.to_string())
        .or_insert_with(|| {
            let (skill_name, skill_icon) = get_skill_name_and_icon(skill_start.skill_id, 0, skill_start.skill_name.to_string());
            Entity {
                name: skill_start.name.to_string(),
                last_update: timestamp,
                skill_stats: SkillStats {
                    casts: 0,
                    ..Default::default()
                },
                skills: HashMap::from([(
                    skill_start.skill_id,
                    Skill {
                        id: skill_start.skill_id,
                        name: skill_name.to_string(),
                        icon: skill_icon.to_string(),
                        casts: 0,
                        ..Default::default()
                    }
                )]),
                ..Default::default()
            }});
    
    entity.last_update = timestamp;
    entity.is_dead = false;
    entity.skill_stats.casts += 1;
    // if skills have different ids but the same name, we group them together
    // dunno if this is right approach xd
    let skill = entity.skills.get_mut(&skill_start.skill_id);
    if skill.is_none() {
        if let Some(skill) = entity.skills.values_mut().find(|s| s.name == skill_start.skill_name.to_string()) {
            skill.casts += 1;
        } else {
            let (skill_name, skill_icon) = get_skill_name_and_icon(skill_start.skill_id, 0, skill_start.skill_name.to_string());
            entity.skills.insert(skill_start.skill_id, Skill {
                id: skill_start.skill_id,
                name: skill_name.to_string(),
                icon: skill_icon.to_string(),
                casts: 1,
                ..Default::default()
            });
        }
    } else {
        skill.unwrap().casts += 1;
    }
}

fn on_skill_stage(_encounter: &mut Encounter, _line: &[&str]) {
}

fn on_damage(reset: &mut bool, encounter: &mut Encounter, timestamp: i64, line: &[&str]) {
    if line.len() < 13 {
        return;
    }
    let mut damage = LogDamage {
        source_id: line[2],
        source_name: if line[3].is_empty() { "Unknown Entity" } else { line[3] },
        skill_id: line[4].parse::<i32>().unwrap_or_default(),
        skill_name: if line[5].is_empty() { "Unknown Skill" } else { line[5] },
        skill_effect_id: line[6].parse::<i32>().unwrap_or_default(),
        skill_effect: line[7],
        target_id: line[8],
        target_name: if line[9].is_empty() { "Unknown Entity" } else { line[9] },
        damage: line[10].parse::<i64>().unwrap_or_default(),
        damage_mod: i32::from_str_radix(line[11], 16).unwrap_or_default(),
        current_hp: line[12].parse::<i64>().unwrap_or_default(),
        max_hp: line[13].parse::<i64>().unwrap_or_default(),
        effects_on_source: HashSet::new(),
        effects_on_target: HashSet::new()
    };

    if line.len() >= 17 {
        for buff in line[14].split(',').step_by(2) {
            if !buff.is_empty() {
                damage.effects_on_target.insert(buff.parse::<i32>().unwrap_or_default());
            }
        }
        for buff in line[15].split(',').step_by(2) {
            if !buff.is_empty() {
                damage.effects_on_source.insert(buff.parse::<i32>().unwrap_or_default());
            }
        }
    }

    let hit_flag = match damage.damage_mod & 0xf {
        0 => HitFlag::NORMAL,
        1 => HitFlag::CRITICAL,
        2 => HitFlag::MISS,
        3 => HitFlag::INVINCIBLE,
        4 => HitFlag::DOT,
        5 => HitFlag::IMMUNE,
        6 => HitFlag::IMMUNE_SILENCED,
        7 => HitFlag::FONT_SILENCED,
        8 => HitFlag::DOT_CRITICAL,
        9 => HitFlag::DODGE,
        10 => HitFlag::REFLECT,
        11 => HitFlag::DAMAGE_SHARE,
        12 => HitFlag::DODGE_HIT,
        13 => HitFlag::MAX,
        _ => { return; }
    };
    let hit_option = match ((damage.damage_mod >> 4) & 0x7) - 1 {
        -1 => HitOption::NONE,
        0 => HitOption::BACK_ATTACK,
        1 => HitOption::FRONTAL_ATTACK,
        2 => HitOption::FLANK_ATTACK,
        3 => HitOption::MAX,
        _ => { return; }
    };

    if *reset {
        soft_reset(encounter);
        *reset = false;
        encounter.reset = false
    }

    if hit_flag == HitFlag::INVINCIBLE {
        return;
    }

    let mut source_entity = encounter.entities.entry(damage.source_name.to_string())
        .or_insert_with(|| Entity {
            id: damage.source_id.to_string(),
            name: damage.source_name.to_string(),
            ..Default::default()
        }).to_owned();

    let mut target_entity = encounter.entities.entry(damage.target_name.to_string())
        .or_insert_with(|| Entity {
            id: damage.target_id.to_string(),
            name: damage.target_name.to_string(),
            current_hp: damage.current_hp,
            max_hp: damage.max_hp,
            ..Default::default()
        }).to_owned();

    source_entity.id = damage.source_id.to_string();
    target_entity.id = damage.target_id.to_string();

    if encounter.fight_start == 0 {
        encounter.fight_start = timestamp;
    }

    target_entity.current_hp = damage.current_hp;
    target_entity.max_hp = damage.max_hp;
    target_entity.last_update = timestamp;
    source_entity.last_update = timestamp;

    if target_entity.entity_type != EntityType::PLAYER && damage.current_hp < 0 {
        damage.damage += damage.current_hp;
    }

    if damage.skill_id == 0 && damage.skill_effect_id != 0 {
        damage.skill_id = damage.skill_effect_id;
        damage.skill_name = damage.skill_effect;
    }

    let skill = source_entity.skills.contains_key(&damage.skill_id);
    let mut skill_id = damage.skill_id;
    if !skill {
        if let Some(skill) = source_entity.skills.values().find(|&s| s.name == damage.skill_name.to_string()) {
            skill_id = skill.id;
        } else {
            let (skill_name, skill_icon) = get_skill_name_and_icon(damage.skill_id, damage.skill_effect_id, damage.skill_name.to_string());
            source_entity.skills.insert(damage.skill_id, Skill {
                id: damage.skill_id,
                name: skill_name.to_string(),
                icon: skill_icon.to_string(),
                casts: 1,
                ..Default::default()
            });
        }
    }

    let skill = source_entity.skills.get_mut(&skill_id).unwrap();

    if damage.skill_name == "Bleed" && hit_flag == HitFlag::DAMAGE_SHARE {
        return;
    }

    let is_crit = hit_flag == HitFlag::CRITICAL || hit_flag == HitFlag::DOT_CRITICAL;
    let is_back_atk = hit_option == HitOption::BACK_ATTACK;
    let is_front_atk = hit_option == HitOption::FRONTAL_ATTACK;

    skill.total_damage += damage.damage;
    if damage.damage > skill.max_damage {
        skill.max_damage = damage.damage;
    }

    source_entity.damage_stats.damage_dealt += damage.damage;
    target_entity.damage_stats.damage_taken += damage.damage;

    // if damage.skill_name != "Bleed" {
    source_entity.skill_stats.hits += 1;
    source_entity.skill_stats.crits += if is_crit { 1 } else { 0 };
    source_entity.skill_stats.back_attacks += if is_back_atk { 1 } else { 0 };
    source_entity.skill_stats.front_attacks += if is_front_atk { 1 } else { 0 };

    skill.hits += 1;
    skill.crits += if is_crit { 1 } else { 0 };
    skill.back_attacks += if is_back_atk { 1 } else { 0 };
    skill.front_attacks += if is_front_atk { 1 } else { 0 };
    // }

    if source_entity.entity_type == EntityType::PLAYER {
        encounter.encounter_damage_stats.total_damage_dealt += damage.damage;
        encounter.encounter_damage_stats.top_damage_dealt = max(encounter.encounter_damage_stats.top_damage_dealt, source_entity.damage_stats.damage_dealt);

        let mut is_buffed_by_support = false;
        let mut is_debuffed_by_support = false;
        for buff_id in damage.effects_on_source.iter() {
            if !encounter.encounter_damage_stats.unknown_buffs.contains(buff_id) && !encounter.encounter_damage_stats.buffs.contains_key(buff_id) {
                if let Some(status_effect) = get_status_effect_data(*buff_id) {
                    encounter.encounter_damage_stats.buffs.insert(*buff_id, status_effect);
                }
            }
            let status_effect = encounter.encounter_damage_stats.buffs.get(buff_id);
            if status_effect.is_some() && !is_buffed_by_support {
                let status_effect = status_effect.unwrap();
                if status_effect.source.skill.is_some() {
                    let skill = status_effect.source.skill.as_ref().unwrap();
                    is_buffed_by_support = (status_effect.buff_category == "classskill" ||
                                        status_effect.buff_category == "identity" ||
                                        status_effect.buff_category == "ability" ) &&
                                        status_effect.target == StatusEffectTarget::PARTY &&
                                        is_support_class_id(skill.class_id);
                }
            }
        }
        for buff_id in damage.effects_on_target.iter() {
            // maybe problem
            if !encounter.encounter_damage_stats.unknown_buffs.contains(buff_id) && !encounter.encounter_damage_stats.debuffs.contains_key(buff_id) {
                if let Some(status_effect) = get_status_effect_data(*buff_id) {
                    encounter.encounter_damage_stats.debuffs.insert(*buff_id, status_effect);
                }
            }
            let status_effect = encounter.encounter_damage_stats.debuffs.get(buff_id);
            if status_effect.is_some() && !is_debuffed_by_support {
                let status_effect = status_effect.unwrap();
                if status_effect.source.skill.is_some() {
                    let skill = status_effect.source.skill.as_ref().unwrap();
                    is_debuffed_by_support = (status_effect.buff_category == "classskill" ||
                                        status_effect.buff_category == "identity" ||
                                        status_effect.buff_category == "ability" ) &&
                                        status_effect.target == StatusEffectTarget::PARTY &&
                                        is_support_class_id(skill.class_id);
                }
            }
        }

        skill.buffed_by_support += if is_buffed_by_support { damage.damage } else { 0 };
        skill.debuffed_by_support += if is_debuffed_by_support { damage.damage } else { 0 };
        source_entity.damage_stats.buffed_by_support += if is_buffed_by_support { damage.damage } else { 0 };
        source_entity.damage_stats.debuffed_by_support += if is_debuffed_by_support { damage.damage } else { 0 };

        for buff_id in damage.effects_on_source.iter() {
            skill.buffed_by.entry(*buff_id).and_modify(|e| *e += damage.damage).or_insert(damage.damage);
            source_entity.damage_stats.buffed_by.entry(*buff_id).and_modify(|e| *e += damage.damage).or_insert(damage.damage);
        }
        for buff_id in damage.effects_on_target.iter() {
            skill.debuffed_by.entry(*buff_id).and_modify(|e| *e += damage.damage).or_insert(damage.damage);
            source_entity.damage_stats.debuffed_by.entry(*buff_id).and_modify(|e| *e += damage.damage).or_insert(damage.damage);
        }
    } 

    if target_entity.entity_type == EntityType::PLAYER {
        encounter.encounter_damage_stats.total_damage_taken += damage.damage;
        encounter.encounter_damage_stats.top_damage_taken = max(encounter.encounter_damage_stats.top_damage_taken, target_entity.damage_stats.damage_taken);
    }

    // update current_boss
    if target_entity.entity_type == EntityType::BOSS {
        encounter.current_boss_name = target_entity.name.to_string();
    } else if target_entity.entity_type == EntityType::UNKNOWN {
        // hard coding this for valtan ghost, and trixion boss
        // if we know the local player, we assume what he is hitting is the boss and we track that instead
        // dunno if want to do this
        if target_entity.max_hp > 1865513010 || target_entity.max_hp == 529402339 || target_entity.max_hp == 285632921 || target_entity.max_hp == 999_999_999 {
            encounter.current_boss_name = target_entity.name.to_string();
        }
    }

    encounter.entities.insert(source_entity.name.to_string(), source_entity);
    encounter.entities.insert(target_entity.name.to_string(), target_entity);

    encounter.last_combat_packet = timestamp;
}

fn on_heal(_encounter: &mut Encounter, _line: &[&str]) {
    println!("Heal");
}

fn on_buff(_encounter: &mut Encounter, _line: &[&str]) {
    println!("Buff");
}

fn on_counterattack(encounter: &mut Encounter, line: &[&str]) {
    let counter = LogCounterAttack {
        id: line[2],
        name: if line[3].is_empty() { "Unknown Entity" } else { line[3] },
        target_id: line[4],
        target_name: if line[5].is_empty() { "Unknown Entity" } else { line[5] }
    };

    let entity = encounter.entities.entry(counter.name.to_string())
        .or_insert_with(|| Entity {
            id: counter.id.to_string(),
            name: counter.name.to_string(),
            entity_type: EntityType::PLAYER,
            skill_stats: SkillStats {
                counters: 1,
                ..Default::default()
            },
            ..Default::default()
        });
    entity.skill_stats.counters += 1;
}

fn is_support_class_id(class_id: i32) -> bool {
    class_id == 105 || class_id == 204 || class_id == 603
}

fn get_status_effect_data(buff_id: i32) -> Option<StatusEffect> {
    let buff = SKILL_BUFF_DATA.get(&buff_id);
    if buff.is_none() || buff.unwrap().icon_show_type == "none" {
        return None;
    }

    let buff = buff.unwrap();
    let buffs = vec![501, 502, 503, 504, 505];
    let buff_category: String;
    if buff.buff_category == "ability" && buffs.contains(&buff.unique_group) {
        buff_category = "dropsofether".to_string();
    } else {
        buff_category = buff.buff_category.to_string();
    }
    let mut status_effect = StatusEffect {
        target: {
            if buff.target == "none" {
                StatusEffectTarget::OTHER
            } else if buff.target == "self" {
                StatusEffectTarget::SELF
            } else {
                StatusEffectTarget::PARTY
            }
        },
        category: buff.category.to_string(),
        buff_category: buff_category.to_string(),
        buff_type: get_status_effect_buff_type_flags(buff),
        unique_group: buff.unique_group,
        source: StatusEffectSource { 
            name: buff.name.to_string(), 
            desc: buff.desc.to_string(), 
            icon: buff.icon.to_string(), 
            ..Default::default() 
        }
    };

    if buff_category == "classkill" || buff_category == "identity" {
        if buff.source_skill.is_some() {
            let buff_source_skill = SKILL_DATA.get(&buff.source_skill.unwrap());
            if buff_source_skill.is_some() {
                status_effect.source.skill = buff_source_skill.cloned();
            }
        } else {
            let skill_id = (buff_id as f32 / 10.0) as i32;
            let buff_source_skill = SKILL_DATA.get(&skill_id);
            if buff_source_skill.is_some() {
                status_effect.source.skill = buff_source_skill.cloned();
            } else {
                let skill_id = (buff.unique_group as f32 / 10.0) as i32;
                let buff_source_skill = SKILL_DATA.get(&skill_id);
                status_effect.source.skill = buff_source_skill.cloned();
            }
        }
    } else if buff_category == "ability" && buff.unique_group != 0 {
        if buff.source_skill.is_some() {
            let buff_source_skill = SKILL_DATA.get(&buff.source_skill.unwrap());
            if buff_source_skill.is_some() {
                status_effect.source.skill = buff_source_skill.cloned();
            }
        } else {
            let skill_id = (buff_id as f32 / 10.0) as i32;
            let buff_source_skill = SKILL_DATA.get(&skill_id);
            if buff_source_skill.is_some() {
                status_effect.source.skill = buff_source_skill.cloned();
            } else {
                let skill_id = (buff.unique_group as f32 / 10.0) as i32;
                let buff_source_skill = SKILL_DATA.get(&skill_id);
                status_effect.source.skill = buff_source_skill.cloned();
            }
        }
    } else if buff_category == "set" && buff.set_name.is_some() {
        status_effect.source.set_name = buff.set_name.clone();
    }

    Some(status_effect)
} 

fn get_status_effect_buff_type_flags(buff: &SkillBuffData) -> u32 {
    let dmg_buffs = vec![
        "weaken_defense",
        "weaken_resistance",
        "skill_damage_amplify",
        "beattacked_damage_amplify",
        "skill_damage_amplify_attack",
        "directional_attack_amplify",
        "instant_stat_amplify",
        "attack_power_amplify",
        "instant_stat_amplify_by_contents",
    ];
    let move_buffs = vec!["move_speed_down", "all_speed_down"];
    let cd_buffs = String::from("reset_cooldown");
    let stagger_buffs = vec!["change_ai_point", "ai_point_amplify"];
    let resource_buffs = String::from("increase_identity_gauge");
    
    let mut buff_type = StatusEffectBuffTypeFlags::NONE;
    if dmg_buffs.contains(&buff.buff_type.as_str()) {
        buff_type |= StatusEffectBuffTypeFlags::DMG;
    } else if move_buffs.contains(&buff.buff_type.as_str()) {
        buff_type |= StatusEffectBuffTypeFlags::MOVESPEED;
    } else if buff.buff_type == cd_buffs {
        buff_type |= StatusEffectBuffTypeFlags::COOLDOWN;
    } else if stagger_buffs.contains(&buff.buff_type.as_str()) {
        buff_type |= StatusEffectBuffTypeFlags::STAGGER;
    } else if buff.buff_type == resource_buffs {
        buff_type |= StatusEffectBuffTypeFlags::RESOURCE;
    }
    buff_type.bits()
}

fn get_skill_name_and_icon(skill_id: i32, skill_effect_id: i32, skill_name: String) -> (String, String) {
    if skill_id == 0 && skill_effect_id == 0 {
        return ("Bleed".to_string(), "buff_168.png".to_string());
    } else if skill_id == 0 {
        if let Some(effect) = SKILL_EFFECT_DATA.get(&skill_effect_id) {
            if effect.item_name.is_some() {
                return (effect.item_name.as_ref().unwrap().to_string(), effect.icon.as_ref().unwrap().to_string());
            }
            if effect.source_skill.is_some() {
                if let Some(skill) = SKILL_DATA.get(&effect.source_skill.unwrap()) {
                    return (skill.name.to_string(), skill.icon.to_string());
                }
            } else {
                if let Some(skill) = SKILL_DATA.get(&((skill_effect_id as f32 / 10.0).floor() as i32)) {
                    return (skill.name.to_string(), skill.icon.to_string());
                }
            }
            return (effect.comment.to_string(), "".to_string());
        } else {
            return (skill_name, "".to_string());
        }
    } else {
        let mut skill = SKILL_DATA.get(&skill_id);
        if skill.is_none() {
            skill = SKILL_DATA.get(&(skill_id - (skill_id as f32 % 10.0) as i32));
            if skill.is_none() {
                return (skill_name, "".to_string());
            }
        }
        let skill = skill.unwrap();
        if skill.summon_source_skill.is_some() {
            let skill = SKILL_DATA.get(&skill.summon_source_skill.unwrap());
            if skill.is_some() {
                let skill = skill.unwrap();
                return (skill.name.to_string() + " (Summon)", skill.icon.to_string());
            } else {
                return (skill_name, "".to_string());
            }
        } else if skill.source_skill.is_some() {
            let skill = SKILL_DATA.get(&skill.source_skill.unwrap());
            if skill.is_some() {
                let skill = skill.unwrap();
                return (skill.name.to_string(), skill.icon.to_string());
            } else {
                return (skill_name, "".to_string());
            }
        } else {
            return (skill.name.to_string(), skill.icon.to_string());
        }
    }
}
