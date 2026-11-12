//! C# `event Action<…>` 的對應：可以註冊多個回呼，觸發時依序呼叫。只在 UI 執行緒使用。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// 註冊回呼後拿到的編號，取消註冊時用（C# 的 `-=`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandlerId(u64);

type Handler<A> = Rc<dyn Fn(&A)>;

pub struct Event<A> {
    handlers: RefCell<Vec<(HandlerId, Handler<A>)>>,
    next: Cell<u64>,
}

impl<A> Default for Event<A> {
    fn default() -> Self {
        Self { handlers: RefCell::new(Vec::new()), next: Cell::new(1) }
    }
}

impl<A> Event<A> {
    /// 註冊（C# `+=`）。
    pub fn subscribe(&self, f: impl Fn(&A) + 'static) -> HandlerId {
        let id = HandlerId(self.next.get());
        self.next.set(id.0 + 1);
        self.handlers.borrow_mut().push((id, Rc::new(f)));
        id
    }

    /// 取消註冊（C# `-=`）。
    pub fn unsubscribe(&self, id: HandlerId) {
        self.handlers.borrow_mut().retain(|(h, _)| *h != id);
    }

    /// 有沒有人註冊（C# `handler is not null`）。
    pub fn has_handlers(&self) -> bool {
        !self.handlers.borrow().is_empty()
    }

    /// 觸發（C# `?.Invoke`）；回呼裡可以再註冊或取消註冊。
    pub fn invoke(&self, arg: &A) {
        let handlers: Vec<Handler<A>> = self.handlers.borrow().iter().map(|(_, h)| h.clone()).collect();
        for h in handlers {
            h(arg);
        }
    }
}
