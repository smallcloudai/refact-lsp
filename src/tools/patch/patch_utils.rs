use std::collections::HashMap;
use std::hash::Hash;


pub fn most_common_value_in_vec<T: Eq + Hash + Copy>(items: Vec<T>) -> Option<T> {
    items.iter()
        .fold(HashMap::new(), |mut acc, &item| {
            *acc.entry(item).or_insert(0) += 1;
            acc
        })
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(item, _)| item)
}
