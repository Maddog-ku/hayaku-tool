//! 對應 C# `Core/FileReader.cs`：讀檔案內容（read_file）的快取與限流。
//! 同一個檔案（同一段）只讀一次；Hayaku 限制每人 10 分鐘最多 30 次（跟 Claude 共用），
//! 這裡自己最多用 24 次，留餘裕給 Claude，超過就請使用者等一下，不會一直打到被擋。
//! 只在 UI 執行緒使用（內部用 `RefCell`）。

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::{NaiveDateTime, TimeDelta};
use hb_api::{FileContent, HayakuApi, McpError};

const MAX_CACHED: usize = 40;
const BUDGET: usize = 24;
const WINDOW: TimeDelta = TimeDelta::minutes(10);

type Key = (String, i32);

pub struct FileReader {
    api: Arc<HayakuApi>,
    cache: RefCell<HashMap<Key, FileContent>>,
    order: RefCell<VecDeque<Key>>,
    calls: RefCell<VecDeque<NaiveDateTime>>,
}

impl FileReader {
    pub fn new(api: Arc<HayakuApi>) -> Self {
        Self { api, cache: RefCell::default(), order: RefCell::default(), calls: RefCell::default() }
    }

    pub fn is_cached(&self, id: &str, offset: i32) -> bool {
        self.cache.borrow().contains_key(&(id.to_owned(), offset))
    }

    pub async fn read(&self, id: &str, offset: i32) -> Result<FileContent, McpError> {
        self.read_at(id, offset, crate::time::now()).await
    }

    async fn read_at(&self, id: &str, offset: i32, now: NaiveDateTime) -> Result<FileContent, McpError> {
        let key = (id.to_owned(), offset);
        if let Some(cached) = self.cache.borrow().get(&key) {
            return Ok(cached.clone());
        }
        {
            let mut calls = self.calls.borrow_mut();
            while calls.front().is_some_and(|t| now - *t > WINDOW) {
                calls.pop_front();
            }
            if calls.len() >= BUDGET {
                let first = calls.front().copied().unwrap_or(now);
                let minutes = hb_api::work_math::total_minutes(WINDOW - (now - first));
                let wait = (minutes.ceil() as i64).max(1);
                return Err(McpError(format!(
                    "10 分鐘內讀了太多檔案（Hayaku 限制每人 30 次，跟 Claude 共用），約 {wait} 分鐘後再試"
                )));
            }
            calls.push_back(now);
        }
        let content = self.api.read_file(id, offset).await?;
        self.cache.borrow_mut().insert(key.clone(), content.clone());
        let mut order = self.order.borrow_mut();
        order.push_back(key);
        while order.len() > MAX_CACHED {
            if let Some(old) = order.pop_front() {
                self.cache.borrow_mut().remove(&old);
            }
        }
        Ok(content)
    }

    /// 定期清暫存時呼叫：圖片很佔記憶體，整個清掉（限流的紀錄留著）。
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
        self.order.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeTransport, block_on, j};

    #[test]
    fn 同一段只讀一次_超過額度請使用者等() {
        let fake = Arc::new(FakeTransport::default());
        let reader = FileReader::new(Arc::new(HayakuApi::new(fake.clone())));
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 12).unwrap().and_hms_opt(9, 0, 0).unwrap();
        fake.reply(j(r#"{"content":[{"type":"text","text":"hello"}]}"#));
        block_on(reader.read_at("f1", 0, now)).unwrap();
        block_on(reader.read_at("f1", 0, now)).unwrap();
        assert_eq!(1, fake.calls().len());
        assert!(reader.is_cached("f1", 0));
        for i in 0..23 {
            block_on(reader.read_at(&format!("x{i}"), 0, now)).unwrap();
        }
        let err = block_on(reader.read_at("y", 0, now + TimeDelta::minutes(3))).unwrap_err();
        assert_eq!("10 分鐘內讀了太多檔案（Hayaku 限制每人 30 次，跟 Claude 共用），約 7 分鐘後再試", err.0);
        // 10 分鐘後額度回來
        assert!(block_on(reader.read_at("y", 0, now + TimeDelta::minutes(11))).is_ok());
        reader.clear();
        assert!(!reader.is_cached("f1", 0));
    }
}
