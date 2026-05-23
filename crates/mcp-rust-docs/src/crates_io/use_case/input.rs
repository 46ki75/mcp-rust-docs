#[derive(Debug, Clone)]
pub struct SearchCratesUseCaseInput {
    pub query: String,
    pub per_page: Option<u8>,
    pub page: Option<u32>,
}
