#[derive(Debug, Clone)]
pub struct SearchCratesRepositoryOutput {
    pub total: u64,
    pub crates: Vec<RepositoryCrateRecord>,
}

#[derive(Debug, Clone)]
pub struct RepositoryCrateRecord {
    pub name: String,
    pub max_version: String,
    pub max_stable_version: Option<String>,
    pub description: Option<String>,
    pub downloads: u64,
    pub recent_downloads: Option<u64>,
    pub documentation: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub updated_at: String,
}
