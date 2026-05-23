#[derive(Debug, Clone)]
pub struct SearchCratesUseCaseOutput {
    pub total: u64,
    pub page: u32,
    pub per_page: u8,
    pub crates: Vec<CrateSummary>,
}

#[derive(Debug, Clone)]
pub struct CrateSummary {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub downloads: u64,
    pub recent_downloads: Option<u64>,
    pub documentation: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub updated_at: String,
}
