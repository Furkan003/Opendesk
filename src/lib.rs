//! Opendesk session policy, encrypted transport, and explicit file transfer.
//! Desktop capture and input integration live in the executable's platform module.

pub mod file_protocol;
#[cfg(feature = "desktop")]
pub mod installer;
pub mod invite;
pub mod mobile;
pub mod network;
pub mod p2p;
pub mod relay;
pub mod transfer;
#[cfg(feature = "desktop")]
pub mod unattended;
#[cfg(feature = "desktop")]
pub mod update;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Screen,
    Input,
    Clipboard,
    Files,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Pending,
    Active(Vec<Capability>),
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    InvalidTransition,
    PermissionDenied,
}

#[derive(Debug)]
pub struct Session {
    authenticated_peer: String,
    state: SessionState,
}

impl Session {
    /// Caller supplies the identity established by an authenticated transport.
    pub fn new(authenticated_peer: String) -> Self {
        Self {
            authenticated_peer,
            state: SessionState::Pending,
        }
    }

    pub fn peer(&self) -> &str {
        &self.authenticated_peer
    }
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Only the host's local consent controller should call this method.
    pub fn accept(&mut self, capabilities: Vec<Capability>) -> Result<(), SessionError> {
        if self.state != SessionState::Pending {
            return Err(SessionError::InvalidTransition);
        }
        self.state = SessionState::Active(capabilities);
        Ok(())
    }

    pub fn authorize(&self, capability: Capability) -> Result<(), SessionError> {
        match &self.state {
            SessionState::Active(granted) if granted.contains(&capability) => Ok(()),
            _ => Err(SessionError::PermissionDenied),
        }
    }

    pub fn revoke(&mut self, capability: Capability) -> Result<(), SessionError> {
        match &mut self.state {
            SessionState::Active(granted) => {
                granted.retain(|c| *c != capability);
                Ok(())
            }
            _ => Err(SessionError::InvalidTransition),
        }
    }

    pub fn close(&mut self) {
        self.state = SessionState::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_grants_only_selected_operations() {
        let mut session = Session::new("authenticated-peer".into());
        assert_eq!(
            session.authorize(Capability::Screen),
            Err(SessionError::PermissionDenied)
        );
        session.accept(vec![Capability::Screen]).unwrap();
        assert!(session.authorize(Capability::Screen).is_ok());
        assert_eq!(
            session.authorize(Capability::Input),
            Err(SessionError::PermissionDenied)
        );
        assert_eq!(
            session.accept(vec![Capability::Input]),
            Err(SessionError::InvalidTransition)
        );
    }

    #[test]
    fn revocation_and_disconnect_remove_access() {
        let mut session = Session::new("authenticated-peer".into());
        session
            .accept(vec![Capability::Screen, Capability::Input])
            .unwrap();
        session.revoke(Capability::Input).unwrap();
        assert_eq!(
            session.authorize(Capability::Input),
            Err(SessionError::PermissionDenied)
        );
        assert!(session.authorize(Capability::Screen).is_ok());
        session.close();
        assert_eq!(
            session.authorize(Capability::Screen),
            Err(SessionError::PermissionDenied)
        );
        assert_eq!(
            session.accept(vec![Capability::Screen]),
            Err(SessionError::InvalidTransition)
        );
    }
}
