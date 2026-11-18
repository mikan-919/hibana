use super::{R, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

// 実行は1スレッド。Waker が共有するのは再開する仕事の番号だけ
type Ready = Arc<Mutex<VecDeque<usize>>>;
type Job<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

struct Resume {
    ready: Ready,
    id: usize,
}

impl Wake for Resume {
    fn wake(self: Arc<Self>) {
        self.ready.lock().unwrap().push_back(self.id);
    }
}

#[derive(Default)]
pub(super) struct Jobs<'a> {
    ready: Ready,
    waiting: RefCell<HashMap<usize, Job<'a>>>,
    next: Cell<usize>,
}

impl<'a> Jobs<'a> {
    // async 関数や Promise の反応を、最初に止まる所まで進める
    pub(super) fn start(&self, job: impl Future<Output = ()> + 'a) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.poll(id, Box::pin(job));
    }

    pub(super) fn promise(&self, work: impl Future<Output = R<Value>> + 'a) -> Rc<Promise> {
        let promise = Rc::new(Promise::default());
        let done = promise.clone();
        self.start(async move { done.settle(work.await) });
        promise
    }

    pub(super) fn step(&self) -> bool {
        let Some(id) = self.ready.lock().unwrap().pop_front() else {
            return false;
        };
        let job = self
            .waiting
            .borrow_mut()
            .remove(&id)
            .expect("再開を待つ仕事");
        self.poll(id, job);
        true
    }

    fn poll(&self, id: usize, mut job: Job<'a>) {
        let waker = Waker::from(Arc::new(Resume {
            ready: self.ready.clone(),
            id,
        }));
        if job
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
        {
            self.waiting.borrow_mut().insert(id, job);
        }
    }
}

#[derive(Debug)]
enum State {
    Pending(Vec<Waker>),
    Settled(R<Value>),
}

#[derive(Debug)]
pub(super) struct Promise(RefCell<State>);

impl Default for Promise {
    fn default() -> Self {
        Self(RefCell::new(State::Pending(Vec::new())))
    }
}

impl Promise {
    pub(super) fn settle(&self, result: R<Value>) {
        let mut state = self.0.borrow_mut();
        if let State::Pending(waiters) = &mut *state {
            for waiter in std::mem::take(waiters) {
                waiter.wake();
            }
            *state = State::Settled(result);
        }
    }

    pub(super) fn result(&self) -> Option<R<Value>> {
        match &*self.0.borrow() {
            State::Settled(result) => Some(result.clone()),
            State::Pending(_) => None,
        }
    }

    // 完了済みでも、await の続きや then の反応は仕事の並びの末尾に積む
    pub(super) async fn wait(&self) -> R<Value> {
        let mut waiting = false;
        poll_fn(|cx| {
            if waiting {
                Poll::Ready(self.result().expect("Promise が完了してから再開する"))
            } else {
                match &mut *self.0.borrow_mut() {
                    State::Pending(waiters) => waiters.push(cx.waker().clone()),
                    State::Settled(_) => cx.waker().wake_by_ref(),
                }
                waiting = true;
                Poll::Pending
            }
        })
        .await
    }
}
