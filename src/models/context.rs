#[derive(Debug, Clone)]
pub struct Context {
    pub tenant_id: u64,
    pub user_id: u64,
    pub workspace_id: u64,
}