use std::collections::HashMap;
use std::net::SocketAddr;

use backup_sync_protocol::{Computer, ComputerId, FolderId, SyncFolder, User, UserId};

#[derive(Debug, Clone)]
pub struct BroadcastMessage {
    pub folder_id: FolderId,
    pub message: String,
}

#[derive(Debug)]
pub struct ConnectedClient {
    pub user_id: Option<UserId>,
    pub computer_id: Option<ComputerId>,
    pub addr: SocketAddr,
}

#[derive(Debug, Default)]
pub struct ServerState {
    pub users: HashMap<UserId, User>,
    pub connections: HashMap<SocketAddr, ConnectedClient>,
    /// Maps (`user_id`, `computer_id`) to socket address for routing
    pub computer_connections: HashMap<(UserId, ComputerId), SocketAddr>,
    /// Pending operations per folder: `folder_id` -> (`operation_id`, `pending_acks`)
    pub pending_operations: HashMap<FolderId, HashMap<u64, usize>>,
    pub operation_counter: u64,
}

impl ServerState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_operation_id(&mut self) -> u64 {
        self.operation_counter += 1;
        self.operation_counter
    }

    pub fn get_or_create_user(&mut self, user_id: &UserId) -> &mut User {
        self.users.entry(user_id.clone()).or_insert_with(|| User {
            id: user_id.clone(),
            name: user_id.clone(),
            computers: Vec::new(),
            sync_folders: Vec::new(),
        })
    }

    #[must_use]
    pub fn get_user(&self, user_id: &UserId) -> Option<&User> {
        self.users.get(user_id)
    }

    pub fn get_user_mut(&mut self, user_id: &UserId) -> Option<&mut User> {
        self.users.get_mut(user_id)
    }

    #[must_use]
    pub fn get_folder(&self, user_id: &UserId, folder_id: &FolderId) -> Option<&SyncFolder> {
        self.users
            .get(user_id)?
            .sync_folders
            .iter()
            .find(|f| &f.id == folder_id)
    }

    pub fn get_folder_mut(
        &mut self,
        user_id: &UserId,
        folder_id: &FolderId,
    ) -> Option<&mut SyncFolder> {
        self.users
            .get_mut(user_id)?
            .sync_folders
            .iter_mut()
            .find(|f| &f.id == folder_id)
    }

    #[must_use]
    pub fn is_folder_synced(&self, user_id: &UserId, folder_id: &FolderId) -> bool {
        self.get_folder(user_id, folder_id)
            .is_some_and(|f| f.is_synced && f.pending_operations == 0)
    }

    pub fn set_computer_online(
        &mut self,
        user_id: &UserId,
        computer_id: &ComputerId,
        online: bool,
    ) {
        if let Some(user) = self.users.get_mut(user_id)
            && let Some(computer) = user.computers.iter_mut().find(|c| &c.id == computer_id)
        {
            computer.online = online;
        }
    }

    pub fn register_connection(&mut self, addr: SocketAddr) {
        self.connections.insert(
            addr,
            ConnectedClient {
                user_id: None,
                computer_id: None,
                addr,
            },
        );
    }

    pub fn remove_connection(&mut self, addr: &SocketAddr) -> Option<ConnectedClient> {
        self.connections.remove(addr)
    }

    #[must_use]
    pub fn get_connection(&self, addr: &SocketAddr) -> Option<&ConnectedClient> {
        self.connections.get(addr)
    }

    pub fn get_connection_mut(&mut self, addr: &SocketAddr) -> Option<&mut ConnectedClient> {
        self.connections.get_mut(addr)
    }

    pub fn authenticate_connection(
        &mut self,
        addr: &SocketAddr,
        user_id: UserId,
        computer_id: ComputerId,
    ) -> Result<(), &'static str> {
        // Check if computer exists for this user
        let computer_exists = self
            .get_user(&user_id)
            .is_some_and(|u| u.computers.iter().any(|c| c.id == computer_id));

        if !computer_exists {
            return Err("Computer not registered for user");
        }

        if let Some(conn) = self.connections.get_mut(addr) {
            conn.user_id = Some(user_id.clone());
            conn.computer_id = Some(computer_id.clone());
        }

        self.computer_connections
            .insert((user_id.clone(), computer_id.clone()), *addr);
        self.set_computer_online(&user_id, &computer_id, true);

        Ok(())
    }

    pub fn register_computer(&mut self, user_id: &UserId, computer: Computer) -> bool {
        if let Some(user) = self.get_user_mut(user_id) {
            user.computers.push(computer);
            true
        } else {
            false
        }
    }

    pub fn create_sync_folder(&mut self, user_id: &UserId, folder: SyncFolder) -> bool {
        if let Some(user) = self.get_user_mut(user_id) {
            user.sync_folders.push(folder);
            true
        } else {
            false
        }
    }

    pub fn join_sync_folder(
        &mut self,
        user_id: &UserId,
        folder_id: &FolderId,
        computer_id: &ComputerId,
    ) -> Option<SyncFolder> {
        if let Some(folder) = self.get_folder_mut(user_id, folder_id) {
            if !folder.backup_computers.contains(computer_id) {
                folder.backup_computers.push(computer_id.clone());
                folder.is_synced = false;
            }
            Some(folder.clone())
        } else {
            None
        }
    }

    pub fn leave_sync_folder(
        &mut self,
        user_id: &UserId,
        folder_id: &FolderId,
        computer_id: &ComputerId,
    ) {
        if let Some(folder) = self.get_folder_mut(user_id, folder_id) {
            folder.backup_computers.retain(|c| c != computer_id);
        }
    }

    #[must_use]
    pub fn is_origin(
        &self,
        user_id: &UserId,
        folder_id: &FolderId,
        computer_id: &ComputerId,
    ) -> bool {
        self.get_folder(user_id, folder_id)
            .is_some_and(|f| &f.origin_computer == computer_id)
    }

    #[must_use]
    pub fn is_backup(
        &self,
        user_id: &UserId,
        folder_id: &FolderId,
        computer_id: &ComputerId,
    ) -> bool {
        self.get_folder(user_id, folder_id)
            .is_some_and(|f| f.backup_computers.contains(computer_id))
    }

    pub fn switch_origin(
        &mut self,
        user_id: &UserId,
        folder_id: &FolderId,
        new_origin: &ComputerId,
    ) -> Result<(), &'static str> {
        if !self.is_folder_synced(user_id, folder_id) {
            return Err("Folder has pending operations and is not fully synced");
        }

        if !self.is_backup(user_id, folder_id, new_origin) {
            return Err("Only backup computers can request to become origin");
        }

        if let Some(folder) = self.get_folder_mut(user_id, folder_id) {
            let old_origin = folder.origin_computer.clone();
            folder.origin_computer = new_origin.clone();
            folder.backup_computers.retain(|c| c != new_origin);
            folder.backup_computers.push(old_origin);
            Ok(())
        } else {
            Err("Folder not found")
        }
    }

    pub fn increment_pending_operations(&mut self, user_id: &UserId, folder_id: &FolderId) {
        if let Some(folder) = self.get_folder_mut(user_id, folder_id) {
            folder.pending_operations += 1;
            folder.is_synced = false;
        }
    }

    #[must_use]
    pub fn get_backup_count(&self, user_id: &UserId, folder_id: &FolderId) -> usize {
        self.get_folder(user_id, folder_id)
            .map_or(0, |f| f.backup_computers.len())
    }

    pub fn track_operation(
        &mut self,
        folder_id: &FolderId,
        operation_id: u64,
        backup_count: usize,
    ) {
        self.pending_operations
            .entry(folder_id.clone())
            .or_default()
            .insert(operation_id, backup_count);
    }

    #[must_use]
    pub fn should_receive_broadcast(&self, addr: &SocketAddr, folder_id: &FolderId) -> bool {
        if let Some(conn) = self.connections.get(addr)
            && let (Some(user_id), Some(computer_id)) = (&conn.user_id, &conn.computer_id)
        {
            return self.is_backup(user_id, folder_id, computer_id);
        }
        false
    }
}

#[must_use]
pub fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{:x}{:x}", duration.as_secs(), duration.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_user(state: &mut ServerState, user_id: &str) {
        state.get_or_create_user(&user_id.to_string());
    }

    #[test]
    fn test_get_or_create_user() {
        let mut state = ServerState::new();

        create_test_user(&mut state, "user1");
        let user = state.get_user(&"user1".to_string()).unwrap();
        assert_eq!(user.id, "user1");
        assert_eq!(user.name, "user1");
        assert!(user.computers.is_empty());
        assert!(user.sync_folders.is_empty());

        // Getting same user should return existing
        let user2 = state.get_or_create_user(&"user1".to_string());
        assert_eq!(user2.id, "user1");
    }

    #[test]
    fn test_register_computer() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let computer = Computer {
            id: "comp1".to_string(),
            name: "My Computer".to_string(),
            online: false,
        };

        assert!(state.register_computer(&"user1".to_string(), computer));

        let user = state.get_user(&"user1".to_string()).unwrap();
        assert_eq!(user.computers.len(), 1);
        assert_eq!(user.computers[0].id, "comp1");
    }

    #[test]
    fn test_register_computer_nonexistent_user() {
        let mut state = ServerState::new();

        let computer = Computer {
            id: "comp1".to_string(),
            name: "My Computer".to_string(),
            online: false,
        };

        assert!(!state.register_computer(&"nonexistent".to_string(), computer));
    }

    #[test]
    fn test_create_sync_folder() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![],
            is_synced: true,
            pending_operations: 0,
        };

        assert!(state.create_sync_folder(&"user1".to_string(), folder));

        let user = state.get_user(&"user1".to_string()).unwrap();
        assert_eq!(user.sync_folders.len(), 1);
        assert_eq!(user.sync_folders[0].id, "folder1");
    }

    #[test]
    fn test_join_sync_folder() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![],
            is_synced: true,
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        let result = state.join_sync_folder(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp2".to_string(),
        );

        assert!(result.is_some());
        let folder = result.unwrap();
        assert!(folder.backup_computers.contains(&"comp2".to_string()));
        assert!(!folder.is_synced); // Should be marked as not synced
    }

    #[test]
    fn test_leave_sync_folder() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        state.leave_sync_folder(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp2".to_string(),
        );

        let folder = state
            .get_folder(&"user1".to_string(), &"folder1".to_string())
            .unwrap();
        assert!(!folder.backup_computers.contains(&"comp2".to_string()));
    }

    #[test]
    fn test_is_folder_synced() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![],
            is_synced: true,
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        assert!(state.is_folder_synced(&"user1".to_string(), &"folder1".to_string()));

        // Mark as not synced
        if let Some(f) = state.get_folder_mut(&"user1".to_string(), &"folder1".to_string()) {
            f.is_synced = false;
        }
        assert!(!state.is_folder_synced(&"user1".to_string(), &"folder1".to_string()));
    }

    #[test]
    fn test_switch_origin_success() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        let result = state.switch_origin(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp2".to_string(),
        );

        assert!(result.is_ok());
        let folder = state
            .get_folder(&"user1".to_string(), &"folder1".to_string())
            .unwrap();
        assert_eq!(folder.origin_computer, "comp2");
        assert!(folder.backup_computers.contains(&"comp1".to_string()));
        assert!(!folder.backup_computers.contains(&"comp2".to_string()));
    }

    #[test]
    fn test_switch_origin_not_synced() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: false, // Not synced
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        let result = state.switch_origin(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp2".to_string(),
        );

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Folder has pending operations and is not fully synced"
        );
    }

    #[test]
    fn test_switch_origin_not_backup() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        let result = state.switch_origin(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp3".to_string(), // Not a backup
        );

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Only backup computers can request to become origin"
        );
    }

    #[test]
    fn test_set_computer_online() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let computer = Computer {
            id: "comp1".to_string(),
            name: "My Computer".to_string(),
            online: false,
        };
        state.register_computer(&"user1".to_string(), computer);

        state.set_computer_online(&"user1".to_string(), &"comp1".to_string(), true);

        let user = state.get_user(&"user1".to_string()).unwrap();
        assert!(user.computers[0].online);
    }

    #[test]
    fn test_operation_counter() {
        let mut state = ServerState::new();

        assert_eq!(state.next_operation_id(), 1);
        assert_eq!(state.next_operation_id(), 2);
        assert_eq!(state.next_operation_id(), 3);
    }

    #[test]
    fn test_connection_management() {
        let mut state = ServerState::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        state.register_connection(addr);
        assert!(state.get_connection(&addr).is_some());

        let conn = state.remove_connection(&addr);
        assert!(conn.is_some());
        assert!(state.get_connection(&addr).is_none());
    }

    #[test]
    fn test_authenticate_connection() {
        let mut state = ServerState::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        create_test_user(&mut state, "user1");
        let computer = Computer {
            id: "comp1".to_string(),
            name: "My Computer".to_string(),
            online: false,
        };
        state.register_computer(&"user1".to_string(), computer);
        state.register_connection(addr);

        let result = state.authenticate_connection(&addr, "user1".to_string(), "comp1".to_string());

        assert!(result.is_ok());

        let conn = state.get_connection(&addr).unwrap();
        assert_eq!(conn.user_id, Some("user1".to_string()));
        assert_eq!(conn.computer_id, Some("comp1".to_string()));

        // Computer should be online
        let user = state.get_user(&"user1".to_string()).unwrap();
        assert!(user.computers[0].online);
    }

    #[test]
    fn test_authenticate_connection_invalid_computer() {
        let mut state = ServerState::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        create_test_user(&mut state, "user1");
        state.register_connection(addr);

        let result =
            state.authenticate_connection(&addr, "user1".to_string(), "nonexistent".to_string());

        assert!(result.is_err());
    }

    #[test]
    fn test_is_origin_and_is_backup() {
        let mut state = ServerState::new();
        create_test_user(&mut state, "user1");

        let folder = SyncFolder {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        };
        state.create_sync_folder(&"user1".to_string(), folder);

        assert!(state.is_origin(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp1".to_string()
        ));
        assert!(!state.is_origin(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp2".to_string()
        ));

        assert!(state.is_backup(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp2".to_string()
        ));
        assert!(!state.is_backup(
            &"user1".to_string(),
            &"folder1".to_string(),
            &"comp1".to_string()
        ));
    }
}
