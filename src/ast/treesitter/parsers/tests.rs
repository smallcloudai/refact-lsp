use std::collections::{HashMap, HashSet};
use std::collections::VecDeque;

use itertools::Itertools;
use similar::DiffableStr;
use uuid::Uuid;

use crate::ast::treesitter::ast_instance_structs::AstSymbolInstanceArc;

mod rust;
mod python;
mod java;
mod cpp;
mod ts;
mod js;

pub(crate) fn print(symbols: &Vec<AstSymbolInstanceArc>, code: &str) {
    let guid_to_symbol_map = symbols.iter()
        .map(|s| (s.clone().read().guid().clone(), s.clone())).collect::<HashMap<_, _>>();
    let sorted = symbols.iter().sorted_by_key(|x| x.read().full_range().start_byte).collect::<Vec<_>>();
    let mut used_guids: HashSet<Uuid> = Default::default();

    for sym in sorted {
        let guid = sym.read().guid().clone();
        if used_guids.contains(&guid) {
            continue;
        }
        let caller_guid = sym.read().get_caller_guid().clone();
        let mut name = sym.read().name().to_string();
        if let Some(caller_guid) = caller_guid {
            if guid_to_symbol_map.contains_key(&caller_guid) {
                name = format!("{} -> {}", name, caller_guid.to_string().slice(0..6));
            }
        }
        let full_range = sym.read().full_range().clone();
        let range = full_range.start_byte..full_range.end_byte;
        println!("{0} {1} [{2}]", guid.to_string().slice(0..6), name, code.slice(range).lines().collect::<Vec<_>>().first().unwrap());
        used_guids.insert(guid.clone());
        let mut candidates: VecDeque<(i32, Uuid)> = VecDeque::from_iter(sym.read().childs_guid().iter().map(|x| (4, x.clone())));
        while let Some((offest, cand)) = candidates.pop_front() {
            used_guids.insert(cand.clone());
            if let Some(sym_l) = guid_to_symbol_map.get(&cand) {
                let caller_guid = sym_l.read().get_caller_guid().clone();
                let mut name = sym_l.read().name().to_string();
                if let Some(caller_guid) = caller_guid {
                    if guid_to_symbol_map.contains_key(&caller_guid) {
                        name = format!("{} -> {}", name, caller_guid.to_string().slice(0..6));
                    }
                }
                let full_range = sym_l.read().full_range().clone();
                let range = full_range.start_byte..full_range.end_byte;
                println!("{0} {1} {2} [{3}]", cand.to_string().slice(0..6), str::repeat(" ", offest as usize), name, code.slice(range).lines().collect::<Vec<_>>().first().unwrap());
                let mut new_candidates = VecDeque::from_iter(sym_l.read().childs_guid().iter().map(|x| (offest + 2, x.clone())));
                new_candidates.extend(candidates.clone());
                candidates = new_candidates;
            }
        }
    }
}
