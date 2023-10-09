
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::Duration;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::mpsc::{channel,Receiver, Sender};
use futures::task::ArcWake;


pub type TaskId = usize;

struct Task {
    future: Pin<Box<dyn Future<Output = ()> + 'static>>,
}


struct MpscWaker(TaskId, Arc<Mutex<Sender<TaskId>>>);
impl MpscWaker {
    fn waker(task_id: TaskId, tx: Sender<TaskId>) -> Waker {
        futures::task::waker(Arc::new(MpscWaker(task_id, Arc::new(Mutex::new(tx)))))
    }
}
impl ArcWake for MpscWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let MpscWaker(task_id, ref tx) = **arc_self;
        tx.lock().unwrap().send(task_id).unwrap();
    }
}

impl Task {
    fn new(f: impl Future<Output = ()> + 'static) -> Self {
        Self {
            future: Box::pin(f),
        }
    }
    /// このタスクを poll して Ready を返すと、このタスクは待機キューから削除される
    fn poll(&mut self, waker: Waker) -> Poll<()> {
        let mut ctx = Context::from_waker(&waker);
        match Future::poll(self.future.as_mut(), &mut ctx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(()) => Poll::Ready(()),
        }
    }
}
pub struct Timeout{
    th: Option<JoinHandle<()>>,
    state: Arc<Mutex<Poll<()>>>,
    waker: Arc<Mutex<Option<Waker>>>,
}
impl Timeout{
    pub fn set(duraction:Duration)->Self{
        let waker = Arc::new(Mutex::new(None::<Waker>));    
        let state = Arc::new(Mutex::new(Poll::Pending));
        let w = waker.clone();
        let s = state.clone();
        let th = std::thread::spawn(move||{
            std::thread::sleep(duraction);
            let mut state = s.lock().unwrap();
            *state = Poll::Ready(());
            if let Some(waker) = &*w.lock().unwrap(){
                waker.wake_by_ref();
            }
        });
        Self{
            th: Some(th),
            state,
            waker,
        }
    }
}
#[derive(Clone)]
pub struct Runtime {
    tx: Sender<TaskId>,
    task_id_counter: Rc<Cell<TaskId>>,
    wait_tasks: Rc<RefCell<BTreeMap<TaskId, Task>>>,
    run_queue: Arc<Mutex<Receiver<TaskId>>>,
}
impl Runtime {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            task_id_counter: Rc::new(Cell::new(0)),
            wait_tasks: Rc::new(RefCell::new(BTreeMap::new())),
            run_queue: Arc::new(Mutex::new(rx)),
        }
    }

    pub fn spawn(&self, f: impl Future<Output = ()> + 'static) {
        let task_id = self.task_id_counter.get();
        self.task_id_counter.set(task_id + 1);
        let waker = MpscWaker::waker(task_id, self.tx.clone());
        let mut task = Task::new(f);

        match task.poll(waker) {
            Poll::Ready(()) => {}
            Poll::Pending => {

                self.wait_tasks.borrow_mut().insert(task_id, task);
            }
        }
    }

    pub fn run(&self, f: impl Future<Output = ()> + 'static) {
        self.spawn(f);
        loop {
            ();
            let task_id = { self.run_queue.lock().unwrap() }.recv().unwrap();
            let mut task = {

              self.wait_tasks.borrow_mut().remove(&task_id).unwrap()
            };

            let waker = MpscWaker::waker(task_id, self.tx.clone());
            match task.poll(waker) {

                Poll::Pending => {
                    self.wait_tasks.borrow_mut().insert(task_id, task);
                }

                Poll::Ready(()) => {}
            }

            if self.wait_tasks.borrow_mut().is_empty() {
                break;
            }
        }
    }
}

impl Future for Timeout{
    type Output = ();
    fn poll(self: Pin<&mut Self>, ctx: &mut Context)->Poll<Self::Output>{
        *self.waker.lock().unwrap() = Some(ctx.waker().clone());
        match *self.state.lock().unwrap(){
            Poll::Ready(()) => Poll::Ready(()),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Timeout{
    fn drop(&mut self){
        self.th.take().unwrap().join().unwrap();
    }
}

fn main(){
    use futures::future::join;


    let runtime = Runtime:: new();
    let r = runtime.clone();

    runtime.run(async move{
        let start_at = std::time::Instant::now();
        r.spawn(async move{
            Timeout::set(Duration::from_millis(100)).await;
            println!("100ms : {:?}", start_at.elapsed().as_millis());
        });
        join(
            async{
                Timeout::set(Duration::from_millis(1000)).await;
                println!("1000ms : {:?}", start_at.elapsed().as_millis());
                Timeout::set(Duration::from_millis(500)).await;
                println!("500ms : {:?}", start_at.elapsed().as_millis());
            },
            async{
                Timeout::set(Duration::from_millis(2000)).await;
                println!("2000ms : {:?}", start_at.elapsed().as_millis());
            },
        )
        .await;
        println!("join : {:?}", start_at.elapsed().as_millis());
    });
}