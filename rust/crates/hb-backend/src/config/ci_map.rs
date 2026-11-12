//! 鍵不分大小寫的字典（C# `new Dictionary<string, T>(StringComparer.OrdinalIgnoreCase)`）。
//! 保留第一次放進去時的鍵寫法與加入順序；JSON 跟一般物件一樣。

use indexmap::IndexMap;
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// OrdinalIgnoreCase 的比較鍵（逐字轉大寫；一個字會變成多個字的不轉，跟 .NET 一樣）。
fn fold(key: &str) -> String {
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

#[derive(Debug, Clone, PartialEq)]
pub struct CiMap<V> {
    /// 比較鍵 → (原本的鍵, 值)
    items: IndexMap<String, (String, V)>,
}

impl<V> Default for CiMap<V> {
    fn default() -> Self {
        Self { items: IndexMap::new() }
    }
}

impl<V> CiMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.items.get(&fold(key)).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.items.get_mut(&fold(key)).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.items.contains_key(&fold(key))
    }

    /// 設定值（C# `dict[key] = value`）：已有的鍵保留原本的寫法。
    pub fn insert(&mut self, key: impl Into<String>, value: V) -> Option<V> {
        let key = key.into();
        match self.items.get_mut(&fold(&key)) {
            Some(slot) => Some(std::mem::replace(&mut slot.1, value)),
            None => {
                self.items.insert(fold(&key), (key, value));
                None
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        self.items.shift_remove(&fold(key)).map(|(_, v)| v)
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.items.values().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.items.values().map(|(k, _)| k.as_str())
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.items.values().map(|(_, v)| v)
    }
}

impl<K: Into<String>, V> FromIterator<(K, V)> for CiMap<V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = Self::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl<V: Serialize> Serialize for CiMap<V> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.items.len()))?;
        for (k, v) in self.items.values() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for CiMap<V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V2<V>(std::marker::PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for V2<V> {
            type Value = CiMap<V>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("物件")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = CiMap::new();
                while let Some((k, v)) = access.next_entry::<String, V>()? {
                    map.insert(k, v);
                }
                Ok(map)
            }
        }
        d.deserialize_map(V2(std::marker::PhantomData))
    }
}
