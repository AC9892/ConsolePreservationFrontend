//! Single-connection transport for the opt-in two-thread component experiment.
//! Contains copied bytes and identifiers, never pointers into another thread's stack.
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
struct State {
    client: Option<u32>,
    packet: Option<Vec<u8>>,
    delivered: bool,
    completion: Option<i32>,
    client_returned: bool,
    teardown_allowed: bool,
}
#[derive(Default)]
pub struct Channel {
    state: Mutex<State>,
    changed: Condvar,
}
impl Channel {
    pub fn enqueue(&self, client: u32, packet: Vec<u8>) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap();
        if client == 0 || packet.is_empty() || packet.len() > 16384 || state.client.is_some() {
            return Err("Invalid or duplicate connection request");
        }
        state.client = Some(client);
        state.packet = Some(packet);
        self.changed.notify_all();
        Ok(())
    }
    pub fn wait_ready(&self, timeout: Duration) -> Result<(), &'static str> {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |s| s.packet.is_none())
            .unwrap();
        if state.packet.is_none() {
            Err("Timed out waiting for connection packet")
        } else {
            Ok(())
        }
    }
    pub fn receive(&self) -> Option<(u32, Vec<u8>)> {
        let mut state = self.state.lock().unwrap();
        let packet = state.packet.take()?;
        state.delivered = true;
        Some((state.client.unwrap(), packet))
    }
    pub fn complete(&self, client: u32, status: i32) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.client != Some(client) || !state.delivered || state.completion.is_some() {
            return Err("Completion does not match a delivered request");
        }
        state.completion = Some(status);
        self.changed.notify_all();
        Ok(())
    }
    pub fn wait_completion(&self, client: u32, timeout: Duration) -> Result<i32, &'static str> {
        let initial = self.state.lock().unwrap();
        if initial.client != Some(client) {
            return Err("Completion requested by different client");
        }
        let (state, _) = self
            .changed
            .wait_timeout_while(initial, timeout, |s| s.completion.is_none())
            .unwrap();
        if state.client != Some(client) {
            return Err("Completion requested by different client");
        }
        state
            .completion
            .ok_or("Timed out waiting for native connection completion")
    }
    pub fn allow_teardown(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap();
        if !state.client_returned || state.teardown_allowed {
            return Err("Invalid teardown transition");
        }
        state.teardown_allowed = true;
        self.changed.notify_all();
        Ok(())
    }
    pub fn client_returned(&self, client: u32) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.client != Some(client) || state.completion != Some(0) || state.client_returned {
            return Err("Client return does not match successful completion");
        }
        state.client_returned = true;
        self.changed.notify_all();
        Ok(())
    }
    pub fn wait_client_returned(&self, timeout: Duration) -> Result<(), &'static str> {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |s| !s.client_returned)
            .unwrap();
        if state.client_returned {
            Ok(())
        } else {
            Err("Timed out waiting for native client return")
        }
    }
    pub fn wait_teardown(&self, timeout: Duration) -> Result<(), &'static str> {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |s| !s.teardown_allowed)
            .unwrap();
        if state.teardown_allowed {
            Ok(())
        } else {
            Err("Timed out waiting for session teardown")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_identity_timeouts_and_thread_wakeup() {
        let channel = Channel::default();
        assert!(channel.wait_ready(Duration::ZERO).is_err());
        assert!(channel.complete(2, 0).is_err());
        assert!(channel.allow_teardown().is_err());
        std::thread::scope(|scope| {
            let client = scope.spawn(|| {
                channel.enqueue(2, vec![1, 2, 3]).unwrap();
                assert!(channel.enqueue(2, vec![4]).is_err());
                assert_eq!(channel.wait_completion(2, Duration::from_secs(2)), Ok(0));
                channel.client_returned(2).unwrap();
                channel.wait_teardown(Duration::from_secs(2)).unwrap();
            });
            channel.wait_ready(Duration::from_secs(2)).unwrap();
            assert!(channel.complete(2, 0).is_err());
            assert_eq!(channel.receive(), Some((2, vec![1, 2, 3])));
            assert!(channel.receive().is_none());
            assert!(channel.complete(3, 0).is_err());
            channel.complete(2, 0).unwrap();
            assert!(channel.complete(2, 0).is_err());
            assert!(channel.wait_completion(3, Duration::ZERO).is_err());
            channel
                .wait_client_returned(Duration::from_secs(2))
                .unwrap();
            channel.allow_teardown().unwrap();
            assert!(channel.allow_teardown().is_err());
            client.join().unwrap();
        });
    }
    #[test]
    fn timeouts_and_failed_completion_do_not_establish_a_connection() {
        let channel = Channel::default();
        assert!(channel.enqueue(0, vec![1]).is_err());
        assert!(channel.enqueue(2, vec![0; 16385]).is_err());
        channel.enqueue(2, vec![1]).unwrap();
        assert!(channel.wait_completion(2, Duration::ZERO).is_err());
        assert!(channel.wait_client_returned(Duration::ZERO).is_err());
        assert!(channel.wait_teardown(Duration::ZERO).is_err());
        channel.receive().unwrap();
        channel.complete(2, -1).unwrap();
        assert_eq!(channel.wait_completion(2, Duration::ZERO), Ok(-1));
        assert!(channel.client_returned(2).is_err());
        assert!(channel.allow_teardown().is_err());
    }
}
