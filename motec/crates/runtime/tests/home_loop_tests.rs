//! The home thread waits in the window system's loop while it has nothing to run, and is woken when a task for it is ready.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use contracts::HomeLoop;
use isa::encoding::{encode_callintrinsic, encode_newobj, encode_none, encode_r2, encode_ri, encode_scopeexit, encode_setfield};
use isa::opcode::Opcode::*;
use isa::value::{TypeDescriptor, Value};
use runtime::handlers::sched_intrinsics::{PIN_ON, TASK_PIN_INTRINSIC};
use runtime::{CodeObject, NativeOutcome, Runtime};

#[derive(Default)]
struct Fake {
    waits: AtomicUsize,
    wakes: AtomicUsize,
    pending: Mutex<bool>,
    cv: Condvar,
}

impl HomeLoop for Fake {
    fn wait(&self, timeout: Option<Duration>) {
        self.waits.fetch_add(1, Ordering::SeqCst);
        let mut pending = self.pending.lock().unwrap();
        if !*pending {
            pending = match timeout {
                None => self.cv.wait(pending).unwrap(),
                Some(d) => self.cv.wait_timeout(pending, d).unwrap().0,
            };
        }
        *pending = false;
    }

    fn wake(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        *self.pending.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn is_open(&self) -> bool {
        true
    }
}

#[test]
fn an_idle_home_worker_waits_in_the_hook_and_a_finished_child_wakes_it() {
    let hook = Arc::new(Fake::default());
    contracts::install_home_loop(hook.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut spin: Vec<_> = (0..300_000).map(|_| encode_ri(LOADI, 6, 0)).collect();
        spin.push(encode_none(HALT));
        let child = CodeObject::new(spin, vec![], 8, 0);
        let mut code = vec![encode_ri(LOADI, 1, PIN_ON as u16), encode_callintrinsic(0, TASK_PIN_INTRINSIC, 1), encode_none(SCOPEENTER)];
        code.extend([encode_newobj(0, 0), encode_ri(LOADI, 1, 1), encode_setfield(0, 0, 1), encode_r2(SPAWN, 2, 0)]);
        code.extend([encode_scopeexit(10), encode_none(HALT)]);
        let main_code = CodeObject::new(code, vec![], 12, 0);
        let mut rt = Runtime::with_types(vec![main_code, child], vec![TypeDescriptor::function_type(0)]);
        rt.set_native_dispatcher(Arc::new(|_, _, _| NativeOutcome::Done(Value::null())));
        let main = rt.acquire_task(0, 12, None, 0);
        tx.send(rt.run_main_parallel(main, 2).map(|_| ())).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(60)).expect("the run finishes").expect("no fault");
    assert!(hook.waits.load(Ordering::SeqCst) >= 1, "the home worker never waited in the hook");
    assert!(hook.wakes.load(Ordering::SeqCst) >= 1, "nothing woke the hook");
}
