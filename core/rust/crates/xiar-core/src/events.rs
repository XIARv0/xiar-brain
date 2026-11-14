// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! In-process event bus. Core components publish events (agent steps, memory writes, UI-variable changes, plug-in
//! state, audit appends); the UI layer and plug-ins subscribe. Topic-prefix matching, bounded per-subscriber queues so
//! a slow or stuck subscriber can never block or exhaust memory in the publisher (it just loses the oldest events and
//! is told how many were dropped).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// dotted topic, e.g. "memory.episodic.insert", "ui.var_changed", "agent.step"
    pub topic: String,
    /// JSON text
    pub payload: String,
}

struct Queue {
    q: Mutex<(VecDeque<Event>, u64, bool)>, // (events, dropped, closed)
    cv: Condvar,
    cap: usize,
}

pub struct Subscription {
    inner: Arc<Queue>,
    prefix: String,
}

impl Subscription {
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Next event, waiting up to `timeout`. `None` on timeout or when the bus was dropped and the queue is empty.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        let mut g = self.inner.q.lock().unwrap();
        loop {
            if let Some(e) = g.0.pop_front() {
                return Some(e);
            }
            if g.2 {
                return None;
            }
            let (ng, res) = self.inner.cv.wait_timeout(g, timeout).unwrap();
            g = ng;
            if res.timed_out() && g.0.is_empty() {
                return None;
            }
        }
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.inner.q.lock().unwrap().0.pop_front()
    }

    /// Number of events dropped because this subscriber's queue was full. Resets to zero when read.
    pub fn take_dropped(&self) -> u64 {
        std::mem::take(&mut self.inner.q.lock().unwrap().1)
    }
}

#[derive(Default)]
pub struct EventBus {
    subs: Mutex<Vec<(String, std::sync::Weak<Queue>)>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to every topic that starts with `prefix` ("" = everything).
    pub fn subscribe(&self, prefix: &str, capacity: usize) -> Subscription {
        let inner = Arc::new(Queue { q: Mutex::new((VecDeque::new(), 0, false)), cv: Condvar::new(), cap: capacity.max(1) });
        self.subs.lock().unwrap().push((prefix.to_string(), Arc::downgrade(&inner)));
        Subscription { inner, prefix: prefix.to_string() }
    }

    pub fn publish(&self, topic: &str, payload: &str) {
        let event = Event { topic: topic.to_string(), payload: payload.to_string() };
        let mut subs = self.subs.lock().unwrap();
        subs.retain(|(prefix, weak)| {
            let Some(q) = weak.upgrade() else { return false }; // subscriber dropped
            if topic.starts_with(prefix.as_str()) {
                let mut g = q.q.lock().unwrap();
                if g.0.len() >= q.cap {
                    g.0.pop_front();
                    g.1 += 1;
                }
                g.0.push_back(event.clone());
                q.cv.notify_one();
            }
            true
        });
    }

    pub fn subscriber_count(&self) -> usize {
        self.subs.lock().unwrap().iter().filter(|(_, w)| w.strong_count() > 0).count()
    }
}

impl Drop for EventBus {
    fn drop(&mut self) {
        for (_, w) in self.subs.lock().unwrap().iter() {
            if let Some(q) = w.upgrade() {
                q.q.lock().unwrap().2 = true;
                q.cv.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_routing() {
        let bus = EventBus::new();
        let mem = bus.subscribe("memory.", 16);
        let all = bus.subscribe("", 16);
        bus.publish("memory.episodic.insert", "{}");
        bus.publish("ui.var_changed", "{\"var\":\"theme\"}");
        assert_eq!(mem.try_recv().unwrap().topic, "memory.episodic.insert");
        assert!(mem.try_recv().is_none());
        assert_eq!(all.try_recv().unwrap().topic, "memory.episodic.insert");
        assert_eq!(all.try_recv().unwrap().topic, "ui.var_changed");
    }

    #[test]
    fn slow_subscriber_drops_oldest_and_reports_it_without_blocking_publisher() {
        let bus = EventBus::new();
        let s = bus.subscribe("", 3);
        for i in 0..10 {
            bus.publish("t", &i.to_string());
        }
        let got: Vec<String> = std::iter::from_fn(|| s.try_recv()).map(|e| e.payload).collect();
        assert_eq!(got, vec!["7", "8", "9"], "keeps the newest events");
        assert_eq!(s.take_dropped(), 7);
        assert_eq!(s.take_dropped(), 0);
    }

    #[test]
    fn dropped_subscribers_are_cleaned_up() {
        let bus = EventBus::new();
        let s = bus.subscribe("", 4);
        assert_eq!(bus.subscriber_count(), 1);
        drop(s);
        bus.publish("t", "x");
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn blocking_receive_wakes_up_across_threads() {
        let bus = Arc::new(EventBus::new());
        let s = bus.subscribe("agent.", 4);
        let b = bus.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            b.publish("agent.step", "go");
        });
        let e = s.recv_timeout(Duration::from_secs(2)).expect("event");
        assert_eq!(e.payload, "go");
        h.join().unwrap();
        assert!(s.recv_timeout(Duration::from_millis(30)).is_none(), "timeout when nothing arrives");
    }

    #[test]
    fn receiver_unblocks_when_bus_is_dropped() {
        let bus = EventBus::new();
        let s = bus.subscribe("", 4);
        drop(bus);
        assert!(s.recv_timeout(Duration::from_secs(2)).is_none());
    }
}
