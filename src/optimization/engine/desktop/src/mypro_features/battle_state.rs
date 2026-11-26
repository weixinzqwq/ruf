//! Read-only battle state. Each instance belongs to one socket session.
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const ABILITIES: [&str; 6] = ["生命", "攻击", "防御", "特攻", "特防", "速度"];
const CATEGORIES: [&str; 4] = ["equipment", "soul-card", "soul-artifact", "crystal-key"];
const MAX_CONFIG_SLOTS: usize = 64;

fn number(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        if let Some(text) = value.as_str() {
            return text.trim().parse().ok();
        }
        let n = value.as_f64()?;
        (n.is_finite() && n.fract() == 0.0 && n.abs() < (1_u64 << 53) as f64).then_some(n as i64)
    })
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| number(value).map(|n| n.to_string()))
}

fn composite(value: &Value) -> Option<(Vec<i64>, i64)> {
    let parts: Vec<_> = if let Some(raw) = value.as_str() {
        raw.split('#')
            .filter_map(|part| part.trim().parse().ok())
            .collect()
    } else {
        vec![number(value)?]
    };
    let total = parts.iter().copied().try_fold(0_i64, i64::checked_add)?;
    (!parts.is_empty()).then_some((parts, total))
}

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Waiting,
    Roster,
    Active,
    Ended,
}

#[derive(Default)]
pub(super) struct BattleState {
    phase: Phase,
    battle_id: Option<String>,
    self_id: Option<u32>,
    last_command: Option<String>,
    sides: BTreeMap<i64, Vec<Pet>>,
    double_pve: Option<[i64; 2]>,
    too_many_owners: bool,
}

fn active_slot_owner(item: &Value) -> Option<(i64, i64)> {
    let view = &item["battleView"];
    number(&view["pmList"].get(0)?["pmSView"]["uqi"]).filter(|id| *id != 0)?;
    Some((number(&view["slotId"])?, number(&view["pmmId"])?))
}

// Confirm both NPC owners from the PVE opening. Species and instance numbers can
// coincide across owners, so subsequent deltas continue to merge within each owner.
fn double_pve_opponents(param: &Value, player_id: Option<u32>) -> Option<[i64; 2]> {
    if param["pvp"].as_bool()? || number(&param["pt"])? != 1 {
        return None;
    }
    let player = i64::from(player_id?);
    let entries = param["pmmList"].as_array()?;
    if entries.len() != 4 {
        return None;
    }
    let slots: BTreeMap<_, _> = entries
        .iter()
        .map(active_slot_owner)
        .collect::<Option<_>>()?;
    let enemies = [*slots.get(&10)?, *slots.get(&11)?];
    (slots.get(&0) == Some(&player)
        && slots.get(&1) == Some(&player)
        && enemies[0] < 0
        && enemies[1] < 0
        && enemies[0] != enemies[1])
        .then_some(enemies)
}

// A partial capture can begin mid-battle. Only a two-sided action with identified
// active pets on this connection can establish the opponent without a roster.
fn has_both_active_sides(param: &Value, player_id: Option<u32>) -> bool {
    let Some(player) = player_id.map(i64::from) else {
        return false;
    };
    let Some([a, b]) = param["pmmList"].as_array().map(Vec::as_slice) else {
        return false;
    };
    matches!((active_slot_owner(a), active_slot_owner(b)), (Some((_, x)), Some((_, y)))
        if x != y && (x == player || y == player))
}

impl BattleState {
    pub(super) fn observe(
        &mut self,
        cmd: &str,
        param: &Value,
        connection_player_id: Option<u32>,
    ) -> bool {
        if !param.is_object()
            || !matches!(
                cmd,
                "2303" | "2304" | "2401" | "2426" | "2411" | "2413" | "2423" | "2404" | "2309"
            )
        {
            return false;
        }
        let opening = matches!(cmd, "2303" | "2304" | "2401");
        let ending = matches!(cmd, "2404" | "2309");
        if !opening {
            match self.phase {
                Phase::Ended => return false,
                Phase::Waiting
                    if !(matches!(cmd, "2411" | "2413" | "2423")
                        && has_both_active_sides(param, connection_player_id)) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        let id = text(&param["battleUniqueId"]).filter(|id| !id.is_empty());
        let different_battle = self
            .battle_id
            .as_ref()
            .zip(id.as_ref())
            .is_some_and(|(a, b)| a != b);
        if different_battle && !matches!(cmd, "2303" | "2401") {
            return false;
        }
        if ending {
            *self = Self {
                phase: Phase::Ended,
                last_command: Some(cmd.to_owned()),
                ..Self::default()
            };
            return true;
        }
        let wrapped = ["b", "pmmList"].into_iter().flat_map(|key| {
            param[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("battleView"))
        });
        let views: Vec<_> = param
            .get("battleView")
            .into_iter()
            .chain(wrapped)
            .filter_map(|view| {
                Some((
                    number(&view["pmmId"])?,
                    number(&view["slotId"]),
                    view["pmList"].as_array()?,
                    view.get("oldPM"),
                ))
            })
            .collect();
        if views.is_empty() {
            return false;
        }
        let carry_roster = self.phase == Phase::Roster
            && !different_battle
            && views
                .iter()
                .any(|(owner, _, _, _)| self.sides.contains_key(owner));
        if cmd == "2303"
            || (cmd == "2401" && !carry_roster)
            || (opening && self.phase == Phase::Ended)
        {
            *self = Self::default();
        }
        if let Some(id) = id {
            self.battle_id = Some(id);
        }
        if let Some(player_id) = connection_player_id {
            self.self_id = Some(player_id);
        }
        if cmd == "2401" {
            self.double_pve = double_pve_opponents(param, self.self_id);
        }
        self.phase = match cmd {
            "2303" | "2304" if self.phase != Phase::Active => Phase::Roster,
            _ => Phase::Active,
        };
        self.last_command = Some(cmd.to_owned());
        for (owner, slot, pets, old_pet) in views {
            if self.sides.len() >= 3 && !self.sides.contains_key(&owner) {
                self.too_many_owners = true;
                continue;
            }
            let side = self.sides.entry(owner).or_default();
            if cmd == "2413"
                && let Some(old_pet) = old_pet
            {
                // The outgoing pet's final values belong to its instance, never to the new slot occupant.
                merge_pet(side, old_pet, None);
            }
            for (index, pet) in pets.iter().enumerate() {
                merge_pet(side, pet, slot.filter(|_| index == 0));
            }
        }
        true
    }

    pub(super) fn snapshot(&self) -> Value {
        let enemy = self.self_id.and_then(|player| {
            if self.too_many_owners {
                return None;
            }
            if let Some(opponents) = self.double_pve {
                let mut pets = Vec::new();
                for owner in opponents {
                    for pet in self.sides.get(&owner)? {
                        let mut snapshot = pet.snapshot();
                        snapshot["pmmId"] = json!(owner);
                        pets.push(snapshot);
                    }
                }
                return Some(json!({"pmmId":null,"pets":pets}));
            }
            let mut other = self
                .sides
                .iter()
                .filter(|(owner, _)| **owner != i64::from(player));
            let (owner, pets) = other.next()?;
            other.next().is_none().then(
                || json!({"pmmId":owner,"pets":pets.iter().map(Pet::snapshot).collect::<Vec<_>>()}),
            )
        });
        json!({
            "state":match self.phase {Phase::Waiting=>"unknown", Phase::Ended=>"ended", _=>"active"},
            "battleUniqueId":self.battle_id, "selfPmmId":self.self_id,
            "lastCommand":self.last_command, "enemy":enemy
        })
    }
}

#[derive(Default)]
struct Pet {
    uqi: Option<i64>,
    slot: Option<i64>,
    view: Map<String, Value>,
    skills: Vec<Value>,
    loadout: Map<String, Value>,
}

fn merge_pet(side: &mut Vec<Pet>, raw: &Value, slot: Option<i64>) {
    let Some(view) = raw["pmSView"].as_object() else {
        return;
    };
    let uqi = view
        .get("uqi")
        .and_then(number)
        .or_else(|| number(&raw["uqi"]))
        .filter(|id| *id != 0);
    if slot.is_none() && uqi.is_none() {
        return;
    }
    let existing = uqi
        .and_then(|id| side.iter().position(|pet| pet.uqi == Some(id)))
        .or_else(|| {
            slot.and_then(|s| {
                side.iter()
                    .position(|pet| pet.slot == Some(s) && (uqi.is_none() || pet.uqi.is_none()))
            })
        });
    if slot.is_some() {
        for (i, pet) in side.iter_mut().enumerate() {
            if Some(i) != existing && pet.slot == slot {
                pet.slot = None;
            }
        }
    }
    let pet = if let Some(i) = existing {
        &mut side[i]
    } else {
        // A single battle normally has six pets per side. Bound malformed long sessions too.
        if side.len() >= 64 {
            return;
        }
        side.push(Pet::default());
        side.last_mut().unwrap()
    };
    pet.uqi = uqi.or(pet.uqi);
    pet.slot = slot.or(pet.slot);
    for field in [
        "r", "skin", "f", "nm", "nick", "lv", "c0", "s0", "ea0", "ea1", "ea2", "ea3", "ea4", "ea5",
    ] {
        if let Some(value) = view
            .get(field)
            .filter(|v| v.is_string() || v.is_number() || v.is_null())
        {
            pet.view.insert(field.into(), value.clone());
        }
    }
    if let Some(skills) = raw["skills"].as_array() {
        pet.skills = skills
            .iter()
            .take(64)
            .filter_map(|s| {
                let id = number(&s["id"])?;
                let previous = pet.skills.iter().find(|old| old["id"].as_i64() == Some(id));
                let mut skill = previous
                    .cloned()
                    .unwrap_or_else(|| json!({"id":id,"name":null,"pp":null,"cd":null}));
                for field in ["name", "pp"] {
                    if let Some(v) = s.get(field) {
                        skill[field] = json!(text(v))
                    }
                }
                if let Some(cd) = s.get("cd") {
                    skill["cd"] = json!(number(cd))
                }
                Some(skill)
            })
            .collect();
    }
    if let Some(eqp) = raw.get("eqp") {
        pet.loadout.insert("equipment".into(), equipment(eqp));
    }
    if let Some(pcp) = view.get("pcp") {
        update_pcp(&mut pet.loadout, pcp);
    }
    side.retain(|pet| pet.uqi.is_some() || pet.slot.is_some());
}

impl Pet {
    fn snapshot(&self) -> Value {
        let lookup = |key: &str| self.view.get(key).unwrap_or(&Value::Null);
        let race = number(lookup("r"));
        let view_id = ["skin", "f"]
            .iter()
            .find_map(|k| number(lookup(k)).filter(|n| *n > 0))
            .or(race);
        let abilities: Vec<_> = ABILITIES
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let field = format!("ea{i}");
                let (segments, total) = composite(lookup(&field))?;
                Some(json!({"field":field,"name":name,"segments":segments,"total":total}))
            })
            .collect();
        let loadout: Map<_, _> = CATEGORIES
            .iter()
            .map(|&c| {
                (
                    c.to_owned(),
                    self.loadout
                        .get(c)
                        .cloned()
                        .unwrap_or_else(|| group("notProvided", vec![])),
                )
            })
            .collect();
        json!({"uqi":self.uqi, "slotId":self.slot, "raceId":race, "viewId":view_id,
            "name":text(lookup("nm")), "nickname":text(lookup("nick")), "level":number(lookup("lv")),
            "currentHp":composite(lookup("c0")).map(|(_,n)|n), "maximumHp":composite(lookup("s0")).map(|(_,n)|n),
            "abilities":abilities, "skills":self.skills, "loadout":loadout})
    }
}

fn group(state: &str, items: Vec<Value>) -> Value {
    json!({"state":state,"items":items})
}

fn items_group(items: Option<Vec<Value>>) -> Value {
    let Some(items) = items.filter(|items| items.len() <= MAX_CONFIG_SLOTS) else {
        return group("invalid", vec![]);
    };
    let equipped = items
        .iter()
        .any(|item| item["id"].as_i64().is_some_and(|id| id > 0));
    group(if equipped { "known" } else { "empty" }, items)
}

fn equipment(raw: &Value) -> Value {
    if raw.is_null() {
        return group("empty", vec![]);
    }
    let Some(slots) = raw.as_array() else {
        return group("invalid", vec![]);
    };
    let items = slots.iter().take(MAX_CONFIG_SLOTS + 1).map(|entry| {
        if entry.is_null() {
            return Some(json!({"id":0,"instanceId":null,"details":null}));
        }
        let parts: Vec<_> = entry["eq"].as_str()?.splitn(13, ',').collect();
        if parts.len() != 12 || parts[2..].iter().any(|p| p.trim().parse::<i32>().is_err()) {
            return None;
        }
        let instance = parts[0].trim().parse::<i64>().ok()?;
        let id = parts[1].trim().parse::<i32>().ok()?;
        let level = text(&entry["l"]).map(|v| format!("等级：{v}"));
        Some(json!({"id":id,"instanceId":instance,"details":level}))
    });
    items_group(items.collect())
}

fn update_pcp(loadout: &mut Map<String, Value>, raw: &Value) {
    let empty = raw.is_null() || raw.as_str() == Some("");
    let doc = raw.as_str().and_then(|xml| {
        roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                nodes_limit: 1024,
                ..Default::default()
            },
        )
        .ok()
    });
    for (attr, category) in [
        ("soc", "soul-card"),
        ("tt", "soul-artifact"),
        ("ctk", "crystal-key"),
    ] {
        let value = if empty {
            group("empty", vec![])
        } else if let Some(doc) = &doc {
            let Some(raw_attr) = doc.root_element().attribute(attr) else {
                continue;
            };
            pcp_group(attr, raw_attr)
        } else {
            group("invalid", vec![])
        };
        loadout.insert(category.into(), value);
    }
}

fn pcp_group(attr: &str, raw: &str) -> Value {
    if raw.is_empty() {
        return group("empty", vec![]);
    }
    let sep = match attr {
        "soc" => '#',
        "tt" => '|',
        _ => '_',
    };
    let items = raw.split(sep).take(MAX_CONFIG_SLOTS + 1).map(|entry| {
        if entry.is_empty() {
            return Some(json!({"id":0,"instanceId":null,"details":null}));
        }
        let (id, instance, details) = match attr {
            "soc" => {
                let mut parts = entry.split(':');
                let level = parts.next()?;
                (parts.next()?, None, Some(format!("槽位等级：{level}")))
            }
            "tt" => {
                let mut parts = entry.split(';');
                let instance = parts.next()?.trim().parse::<i64>().ok()?;
                (
                    parts.next()?,
                    Some(instance),
                    Some(parts.next()?.to_owned()),
                )
            }
            _ => (entry.split(';').next()?, None, None),
        };
        let id = id.trim().parse::<i32>().ok()?.max(0);
        Some(json!({"id":id,"instanceId":instance,"details":details}))
    });
    items_group(items.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(owner: i64, pets: Value) -> Value {
        json!({"battleView":{"pmmId":owner,"slotId":0,"pmList":pets}})
    }

    fn pet(uqi: i64) -> Value {
        json!({"pmSView":{"uqi":uqi,"r":5393,"nm":"练习亚比","c0":"90#2","s0":110,
            "ea0":"100#10","ea1":"18#3","ea2":20,"ea3":21,"ea4":22,"ea5":23,
            "pcp":"<p soc='10:1#0:0' tt='99;2;1:3' ctk='1_0'/>"},
            "skills":[{"id":4,"name":"练习技能","pp":"4/5","cd":2}],
            "eqp":[null,{"eq":"9,24195,0,0,12,0,0,12,0,0,12,0","l":1}]})
    }

    fn at_slot(owner: i64, slot: i64, pet: Value) -> Value {
        let mut entry = side(owner, json!([pet]));
        entry["battleView"]["slotId"] = json!(slot);
        entry
    }

    #[test]
    fn double_pve_keeps_each_npc_roster_and_updates_only_the_addressed_owner() {
        let mut state = BattleState::default();
        let me = Some(1);
        let own = at_slot(1, 0, pet(1));
        let own_second = at_slot(1, 1, pet(2));
        let mut first = pet(50);
        first["skills"][0]["pp"] = json!("2/5");
        let mut second = pet(50); // The same instance number on a different NPC is independent.
        second["eqp"] = json!([]);
        state.observe("2303", &json!({"pmmList":[own,own_second]}), me);
        state.observe("2304", &at_slot(-3, 11, second), me);
        state.observe("2304", &at_slot(-2, 10, first), me);
        assert!(state.snapshot()["enemy"].is_null());
        let hp = |value| json!({"pmSView":{"uqi":50,"c0":value}});
        let open = json!({"pvp":false,"pt":1,"pmmList":[own,at_slot(-3,11,hp(111)),own_second,at_slot(-2,10,hp(100))]});
        assert!(state.observe("2401", &open, me));
        let enemy = &state.snapshot()["enemy"];
        assert_eq!(enemy["pmmId"], Value::Null);
        let pets = enemy["pets"].as_array().unwrap();
        assert_eq!(pets.len(), 2);
        assert_eq!(pets[0]["slotId"], 10);
        assert_eq!(pets[1]["slotId"], 11);
        assert_eq!(pets[0]["currentHp"], 100);
        assert_eq!(pets[1]["currentHp"], 111);
        assert_eq!(pets[0]["skills"][0]["pp"], "2/5");
        assert_eq!(pets[1]["skills"][0]["pp"], "4/5");
        assert_eq!(pets[0]["loadout"]["equipment"]["state"], "known");
        assert_eq!(pets[1]["loadout"]["equipment"]["state"], "empty");
        assert!(state.observe("2423", &at_slot(-3, 11, hp(0)), me));
        let changed = &state.snapshot()["enemy"]["pets"];
        assert_eq!(changed[0], pets[0]);
        assert_eq!(changed[1]["currentHp"], 0);
        let mut switched = at_slot(-3, 11, pet(51));
        switched["battleView"]["oldPM"] = hp(0);
        state.observe("2413", &switched, me);
        let changed = &state.snapshot()["enemy"]["pets"];
        assert_eq!(changed[0], pets[0]);
        assert_eq!(changed[1]["slotId"], Value::Null);
        assert_eq!(changed[2]["slotId"], 11);
        state.observe("2404", &json!({}), me);
        assert!(state.snapshot()["enemy"].is_null());
        assert!(!state.observe("2413", &switched, me));
        state.observe("2401", &at_slot(-8, 11, pet(80)), me);
        assert_eq!(
            state.snapshot()["enemy"]["pets"].as_array().unwrap().len(),
            1
        );
        assert_eq!(state.snapshot()["enemy"]["pets"][0]["uqi"], 80);
    }

    #[test]
    fn ambiguous_four_slot_battles_do_not_expose_an_enemy() {
        let entries = json!([
            at_slot(1, 0, pet(1)),
            at_slot(1, 1, pet(2)),
            at_slot(-2, 10, pet(3)),
            at_slot(-3, 11, pet(4))
        ]);
        for (pvp, pt, slot_one_owner, right_owner, right_slot, right_uqi) in [
            (json!(true), 1, 1, -3, 11, 4),
            (Value::Null, 1, 1, -3, 11, 4),
            (json!(false), 0, 1, -3, 11, 4),
            (json!(false), 1, 7, -3, 11, 4),
            (json!(false), 1, 1, 7, 11, 4),
            (json!(false), 1, 1, -3, 10, 4),
            (json!(false), 1, 1, -3, 11, 0),
        ] {
            let mut state = BattleState::default();
            let mut open = json!({"pvp":pvp,"pt":pt,"pmmList":entries});
            open["pmmList"][1]["battleView"]["pmmId"] = json!(slot_one_owner);
            open["pmmList"][3] = at_slot(right_owner, right_slot, pet(right_uqi));
            state.observe("2401", &open, Some(1));
            assert!(state.snapshot()["enemy"].is_null());
        }
    }

    #[test]
    fn mid_battle_requires_two_identified_active_sides_and_never_revives_an_ended_battle() {
        let own = side(1, json!([pet(10)]));
        let enemy = side(-2, json!([pet(20)]));
        let pair = json!({"pmmList":[own,enemy]});
        for cmd in ["2411", "2413", "2423"] {
            let mut state = BattleState::default();
            for partial in [
                json!({"pmmList":[enemy]}),
                json!({"pmmList":[own,own]}),
                json!({"pmmList":[enemy,side(-3,json!([pet(30)]))]}),
                json!({"pmmList":[own,side(-2,json!([pet(0)]))]}),
                json!({"pmmList":[own,enemy,side(-3,json!([pet(30)]))]}),
            ] {
                assert!(!state.observe(cmd, &partial, Some(1)));
            }
            assert!(!state.observe(cmd, &pair, None));
            assert!(state.observe(cmd, &pair, Some(1)));
            assert_eq!(state.snapshot()["enemy"]["pets"][0]["uqi"], 20);
            assert!(state.observe("2404", &json!({}), Some(1)));
            assert_eq!(state.snapshot()["state"], "ended");
            assert_eq!(state.snapshot()["enemy"], Value::Null);
            assert!(!state.observe(cmd, &pair, Some(1)));
            assert!(state.observe("2303", &own, Some(1)));
            assert_eq!(state.snapshot()["enemy"], Value::Null);
            assert!(state.observe("2304", &side(-3, json!([pet(30)])), Some(1)));
            let snap = state.snapshot();
            assert_eq!(snap["enemy"]["pets"].as_array().unwrap().len(), 1);
            assert_eq!(snap["enemy"]["pets"][0]["uqi"], 30);
        }
    }

    #[test]
    fn switch_applies_old_pm_before_new_occupant_and_settlement_updates_only_present_fields() {
        let mut state = BattleState::default();
        let me = Some(1);
        let mut change = side(-2, json!([pet(21)]));
        change["battleView"]["oldPM"] = json!({"pmSView":{"uqi":20,"c0":0},
            "skills":[{"id":4,"pp":"0/5","cd":0}]});
        assert!(state.observe("2401", &side(-2, json!([pet(20)])), me));
        assert!(state.observe("2413", &change, me));
        let snap = state.snapshot();
        let pets = snap["enemy"]["pets"].as_array().unwrap();
        assert_eq!(pets.len(), 2);
        assert_eq!(pets[0]["slotId"], Value::Null);
        assert_eq!(pets[0]["currentHp"], 0);
        assert_eq!(pets[0]["maximumHp"], 110);
        assert_eq!(
            pets[0]["skills"][0],
            json!({"id":4,"name":"练习技能","pp":"0/5","cd":0})
        );
        assert_eq!(pets[1]["uqi"], 21);
        assert_eq!(pets[1]["slotId"], 0);
        let damage = json!({"pmSView":{"uqi":21,"c0":17}, "skills":[{"id":4,"cd":0}]});
        assert!(state.observe("2423", &json!({"pmmList":[side(-2,json!([damage]))]}), me));
        let snap = state.snapshot();
        let new = &snap["enemy"]["pets"][1];
        assert_eq!(new["currentHp"], 17);
        assert_eq!(new["maximumHp"], 110);
        assert_eq!(
            new["skills"][0],
            json!({"id":4,"name":"练习技能","pp":"4/5","cd":0})
        );
        assert_eq!(new["loadout"]["equipment"]["state"], "known");
    }

    #[test]
    fn aola_battle_assistant_tracks_solo_pve_and_pvp() {
        for opponent in [2002, -2002] {
            let mut state = BattleState::default();
            let me = Some(1001);
            assert!(state.observe("2303", &side(1001, json!([pet(1)])), me));
            assert!(state.observe(
                "2304",
                &json!({"b":[side(opponent,json!([pet(2),pet(3)]))]}),
                me
            ));
            let active_only = |id| json!({"pmSView":{"uqi":id}});
            assert!(state.observe(
                "2401",
                &json!({"battleUniqueId":"A","pmmList":[
                side(1001,json!([active_only(1)])),side(opponent,json!([active_only(2)]))]}),
                me
            ));
            let snap = state.snapshot();
            assert_eq!(snap["enemy"]["pmmId"], opponent);
            let first = &snap["enemy"]["pets"][0];
            assert_eq!(first["currentHp"], 92);
            assert_eq!(first["maximumHp"], 110);
            assert_eq!(first["abilities"].as_array().unwrap().len(), 6);
            assert_eq!(
                first["abilities"][0],
                json!({"name":"生命","field":"ea0","segments":[100,10],"total":110})
            );
            let configs = &first["loadout"];
            assert_eq!(configs["equipment"]["items"][1]["id"], 24195);
            assert_eq!(configs["soul-card"]["items"][0]["id"], 1);
            assert_eq!(configs["soul-artifact"]["items"][0]["instanceId"], 99);
            assert_eq!(configs["crystal-key"]["items"][0]["id"], 1);

            let switched = json!({"pmSView":{"uqi":"3","c0":0},
                "skills":[{"id":4,"pp":"3/5"}]});
            assert!(state.observe("2411", &json!({"b":[side(opponent,json!([switched]))]}), me));
            let pets = &state.snapshot()["enemy"]["pets"];
            assert_eq!(pets.as_array().unwrap().len(), 2);
            assert_eq!(pets[0]["uqi"], 2);
            assert_eq!(pets[0]["slotId"], Value::Null);
            assert_eq!(pets[1]["uqi"], 3);
            assert_eq!(pets[1]["slotId"], 0);
            assert_eq!(pets[1]["currentHp"], 0);
            assert_eq!(
                pets[1]["skills"][0],
                json!({"id":4,"name":"练习技能","pp":"3/5","cd":2})
            );
            assert_eq!(pets[1]["loadout"]["equipment"]["state"], "known");
            assert!(state.observe("2309", &json!({}), me));
            assert_eq!(state.snapshot()["state"], "ended");
            assert!(!state.observe("2411", &side(opponent, json!([pet(4)])), me));
        }
    }

    #[test]
    fn aola_battle_assistant_distinguishes_omitted_empty_and_invalid_configuration() {
        let mut state = BattleState::default();
        let receive = |state: &mut BattleState, cmd, pet| {
            state.observe(cmd, &side(-2, json!([pet])), Some(1))
        };
        receive(&mut state, "2401", json!({"pmSView":{"uqi":2}}));
        let snap = state.snapshot();
        for c in CATEGORIES {
            assert_eq!(
                snap["enemy"]["pets"][0]["loadout"][c]["state"],
                "notProvided"
            )
        }
        receive(&mut state, "2411", pet(2));
        receive(
            &mut state,
            "2411",
            json!({"pmSView":{"uqi":2,"pcp":"<p ctk=''/>"},"eqp":[]}),
        );
        let configs = &state.snapshot()["enemy"]["pets"][0]["loadout"];
        assert_eq!(configs["equipment"]["state"], "empty");
        assert_eq!(configs["crystal-key"]["state"], "empty");
        assert_eq!(configs["soul-card"]["state"], "known");
        assert_eq!(configs["soul-artifact"]["items"][0]["id"], 2);
        for bad in [
            "<broken",
            "<p/>trailing",
            "<p/><q/>",
            "<!DOCTYPE p [<!ENTITY e '12'>]><p/>",
        ] {
            receive(
                &mut state,
                "2411",
                json!({"pmSView":{"uqi":2,"pcp":bad},"eqp":[{"eq":"9,24195"}]}),
            );
            for c in CATEGORIES {
                assert_eq!(
                    state.snapshot()["enemy"]["pets"][0]["loadout"][c]["state"],
                    "invalid"
                );
            }
        }
        receive(
            &mut state,
            "2411",
            json!({"pmSView":{"uqi":2,"pcp":null},"eqp":null}),
        );
        for c in CATEGORIES {
            assert_eq!(
                state.snapshot()["enemy"]["pets"][0]["loadout"][c]["state"],
                "empty"
            )
        }
    }

    #[test]
    fn aola_battle_assistant_requires_valid_opening_and_player_identity() {
        let mut state = BattleState::default();
        let roster =
            json!({"battleUniqueId":"A","b":[side(1,json!([pet(10)])),side(2,json!([pet(20)]))]});
        assert!(!state.observe("9999", &roster, Some(1)));
        assert!(!state.observe("2401", &Value::Null, Some(1)));
        assert!(!state.observe("2401", &json!({"battleView":{"pmmId":2}}), Some(1)));
        assert!(!state.observe("2404", &json!({}), Some(1)));
        assert!(!state.observe("2411", &roster, Some(1)));
        assert_eq!(state.snapshot()["state"], "unknown");
        assert!(state.observe("2401", &roster, None));
        assert_eq!(state.snapshot()["enemy"], Value::Null);
        assert!(state.observe("2426", &side(2, json!([pet(20)])), Some(1)));
        assert_eq!(state.snapshot()["enemy"]["pmmId"], 2);
        assert!(!state.observe("2404", &json!({"battleUniqueId":"older"}), Some(1)));
        assert!(state.observe("2304", &side(3, json!([pet(30)])), Some(1)));
        assert_eq!(state.snapshot()["enemy"], Value::Null);
        assert!(state.observe("2401", &side(-4, json!([pet(31)])), Some(1)));
        assert_eq!(state.snapshot()["enemy"]["pmmId"], -4);
        assert_eq!(
            state.snapshot()["enemy"]["pets"].as_array().unwrap().len(),
            1
        );
        assert_eq!(state.snapshot()["battleUniqueId"], Value::Null);
        let reset = BattleState::default();
        assert_eq!(reset.snapshot()["selfPmmId"], Value::Null);
        assert_eq!(reset.snapshot()["enemy"], Value::Null);
    }
}
