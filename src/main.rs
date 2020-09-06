use crossbeam::{
    channel::{self, RecvError},
    thread,
};
use rand::{seq::SliceRandom, thread_rng};
use std::{
    collections::HashMap,
    iter::repeat_with,
    sync::{Condvar, Mutex, RwLock},
};

struct State {
    in_deficit: Vec<u64>,
    out_deficit: u64,
    parent: Option<usize>,
    is_terminated: bool,
    system_terminated: bool,
}
impl State {
    fn new(nodes: usize) -> State {
        State {
            in_deficit: vec![0; nodes],
            out_deficit: 0,
            parent: None,
            is_terminated: true,
            system_terminated: false,
        }
    }
}

struct Message {
    payload: Payload,
    by: usize,
}
struct Payload(u64);
struct Signal;

// ben-ari assumes atomicity of each algorithm in a process in a node
fn main() {
    const WORKERS: usize = 3;
    // plus environment node
    const NODES: usize = WORKERS + 1;
    const TARGETS: [&[usize]; NODES] = [&[1], &[1, 2, 3], &[1, 2, 3], &[1, 2, 3]];
    const SOURCES: [&[usize]; NODES] = [&[], &[0, 1, 2, 3], &[1, 2, 3], &[1, 2, 3]];

    let (msg_txs, msg_rxs) = repeat_with(|| channel::unbounded())
        .take(NODES)
        .enumerate()
        .fold(
            (HashMap::new(), HashMap::new()),
            |(mut txs, mut rxs), (i, (tx, rx))| {
                txs.insert(i, tx);
                rxs.insert(i, rx);
                (txs, rxs)
            },
        );
    let (msg_txs, msg_rxs) = (RwLock::new(msg_txs), Mutex::new(msg_rxs));
    let (sig_txs, sig_rxs) = repeat_with(|| channel::unbounded())
        .take(NODES)
        .enumerate()
        .fold(
            (HashMap::new(), HashMap::new()),
            |(mut txs, mut rxs), (i, (tx, rx))| {
                txs.insert(i, tx);
                rxs.insert(i, rx);
                (txs, rxs)
            },
        );
    let (sig_txs, sig_rxs) = (RwLock::new(sig_txs), Mutex::new(sig_rxs));
    thread::scope(|s| {
        s.spawn(|_| {
            let i = 0;
            let (lock, cvar) = (Mutex::new(State::new(NODES)), Condvar::new());
            let sig_rx = sig_rxs.lock().unwrap().remove(&i).unwrap();

            let computation = || {
                let mut state = lock.lock().unwrap();
                for t in TARGETS[i] {
                    msg_txs.read().unwrap()[t]
                        .send(Message {
                            payload: Payload(4),
                            by: i,
                        })
                        .unwrap();
                    state.out_deficit += 1;
                }
                let _state = cvar
                    .wait_while(state, |state| state.out_deficit != 0)
                    .unwrap();
                // disconnecting senders will wake up receivers
                msg_txs.write().unwrap().drain();
                sig_txs.write().unwrap().drain();
            };
            let recv_sig = || {
                while let Ok(_) = sig_rx.recv() {
                    let mut state = lock.lock().unwrap();
                    state.out_deficit -= 1;
                    cvar.notify_one();
                }
            };

            thread::scope(|s| {
                s.spawn(|_| {
                    recv_sig();
                });
                s.spawn(|_| {
                    computation();
                });
            })
            .unwrap();
        });
        for i in 1..NODES {
            let msg_txs = &msg_txs;
            let msg_rxs = &msg_rxs;
            let sig_txs = &sig_txs;
            let sig_rxs = &sig_rxs;
            s.spawn(move |_| {
                let (lock, cvar) = (Mutex::new(State::new(NODES)), Condvar::new());
                let msg_rx = msg_rxs.lock().unwrap().remove(&i).unwrap();
                let sig_rx = sig_rxs.lock().unwrap().remove(&i).unwrap();

                let send_msg = |msg, t| {
                    let state = lock.lock().unwrap();
                    let mut state = cvar
                        .wait_while(state, |state| state.parent.is_none())
                        .unwrap();
                    msg_txs.read().unwrap()[&t].send(msg).unwrap();
                    state.out_deficit += 1;
                };
                let recv_msg = || -> Result<_, RecvError> {
                    let msg = msg_rx.recv()?;
                    let mut state = lock.lock().unwrap();
                    state.is_terminated = false;
                    if state.parent.is_none() {
                        state.parent = Some(msg.by);
                    }
                    state.in_deficit[msg.by] += 1;
                    cvar.notify_one();
                    Ok(msg)
                };
                let recv_sig = || {
                    while let Ok(_) = sig_rx.recv() {
                        let mut state = lock.lock().unwrap();
                        state.out_deficit -= 1;
                        cvar.notify_one();
                    }
                    let mut state = lock.lock().unwrap();
                    state.system_terminated = true;
                    cvar.notify_one();
                };
                let computation = || {
                    let mut rng = thread_rng();
                    while let Ok(Message {
                        payload: Payload(x),
                        ..
                    }) = recv_msg()
                    {
                        if x == 0 {
                            println!("processed leaf computation");
                        } else {
                            for _ in 0..2 {
                                let t = TARGETS[i].choose(&mut rng).copied().unwrap();
                                send_msg(
                                    Message {
                                        payload: Payload(x - 1),
                                        by: i,
                                    },
                                    t,
                                );
                            }
                        }
                        let mut state = lock.lock().unwrap();
                        state.is_terminated = true;
                        cvar.notify_one();
                    }
                };
                let send_sig = || loop {
                    let state = lock.lock().unwrap();
                    let mut state = cvar
                        .wait_while(state, |state| {
                            let in_deficit = state.in_deficit.iter().copied().sum::<u64>();
                            !state.system_terminated
                                && !(in_deficit > 1)
                                && !(in_deficit == 1
                                    && state.is_terminated
                                    && state.out_deficit == 0)
                        })
                        .unwrap();
                    let in_deficit = state.in_deficit.iter().copied().sum::<u64>();
                    if state.system_terminated {
                        break;
                    } else if in_deficit > 1 {
                        let s = SOURCES[i]
                            .iter()
                            .copied()
                            .find(|&s| {
                                let d = state.in_deficit[s];
                                d > 1 || (d == 1 && Some(s) != state.parent)
                            })
                            .unwrap();
                        sig_txs.read().unwrap()[&s].send(Signal).unwrap();
                        state.in_deficit[s] -= 1;
                    } else {
                        let parent = state.parent.take().unwrap();
                        sig_txs.read().unwrap()[&parent].send(Signal).unwrap();
                        state.in_deficit[parent] = 0;
                    }
                };

                thread::scope(|s| {
                    s.spawn(|_| {
                        recv_sig();
                    });
                    s.spawn(|_| {
                        send_sig();
                    });
                    s.spawn(|_| {
                        computation();
                    });
                })
                .unwrap();
            });
        }
    })
    .unwrap();
}
