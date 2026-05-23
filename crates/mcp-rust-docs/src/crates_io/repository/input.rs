#[derive(Debug, Clone)]
pub struct SearchCratesRepositoryInput {
    pub query: String,
    pub per_page: u8,
    pub page: u32,
}
