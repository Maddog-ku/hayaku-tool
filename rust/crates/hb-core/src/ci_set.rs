//! 不分大小寫的字串集合（C# `new HashSet<string>(StringComparer.OrdinalIgnoreCase)`）。
//! 保留第一次加入時的寫法與加入順序（C# 的 HashSet 沒刪過元素時也是照加入順序列舉）。

use indexmap::IndexMap;

/// OrdinalIgnoreCase 的比較鍵（逐字轉大寫；一個字會變成多個字的不轉，跟 .NET 一樣）。
pub fn fold(key: &str) -> String {
    key.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

/// `string.Equals(a, b, StringComparison.OrdinalIgnoreCase)`。
pub fn eq_ic(a: &str, b: &str) -> bool {
    a == b || fold(a) == fold(b)
}

/// `StringComparer.OrdinalIgnoreCase.Compare`。
pub fn cmp_ic(a: &str, b: &str) -> std::cmp::Ordering {
    fold(a).cmp(&fold(b))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CiSet {
    /// 比較鍵 → 原本的寫法
    items: IndexMap<String, String>,
}

impl CiSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.items.contains_key(&fold(key))
    }

    /// 加入；已經有（不分大小寫）回 false，保留原本的寫法。
    pub fn insert(&mut self, key: impl Into<String>) -> bool {
        let key = key.into();
        let folded = fold(&key);
        if self.items.contains_key(&folded) {
            return false;
        }
        self.items.insert(folded, key);
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.items.shift_remove(&fold(key)).is_some()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// 集合裡的寫法（`after.First(k => Equals(k, key, OrdinalIgnoreCase))`）。
    pub fn get(&self, key: &str) -> Option<&str> {
        self.items.get(&fold(key)).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.items.values().map(String::as_str)
    }

    /// `RemoveWhere(k => !keep(k))`。
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.items.retain(|_, v| keep(v));
    }

    /// `UnionWith`。
    pub fn extend<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, keys: I) {
        for k in keys {
            self.insert(k);
        }
    }

    /// `IntersectWith`。
    pub fn intersect_with<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) {
        let other: CiSet = keys.into_iter().collect();
        self.items.retain(|k, _| other.items.contains_key(k));
    }

    /// 照序數（ordinal）排序的清單（C# `OrderBy(k => k)`）。
    pub fn sorted(&self) -> Vec<String> {
        let mut list: Vec<String> = self.items.values().cloned().collect();
        list.sort();
        list
    }

    pub fn to_vec(&self) -> Vec<String> {
        self.items.values().cloned().collect()
    }
}

impl<S: Into<String>> FromIterator<S> for CiSet {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        let mut set = Self::new();
        set.extend(iter);
        set
    }
}

impl<'a> IntoIterator for &'a CiSet {
    type Item = &'a str;
    type IntoIter = std::iter::Map<indexmap::map::Values<'a, String, String>, fn(&'a String) -> &'a str>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.values().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 不分大小寫_保留第一次的寫法() {
        let mut set: CiSet = ["F2P-1", "f2p-1", "EG-3"].into_iter().collect();
        assert_eq!(set.len(), 2);
        assert!(set.contains("F2P-1"));
        assert_eq!(set.get("f2p-1"), Some("F2P-1"));
        assert!(!set.insert("eg-3"));
        set.intersect_with(["eg-3"]);
        assert_eq!(set.to_vec(), vec!["EG-3".to_owned()]);
        assert!(eq_ic("abc", "ABC"));
    }
}
