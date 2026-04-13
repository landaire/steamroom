use crate::depot::AppId;
use crate::depot::PackageId;

#[derive(Clone, Debug)]
pub struct AccessToken {
    pub app_id: AppId,
    pub token: u64,
}

#[derive(Clone, Debug)]
pub struct AppInfo {
    pub app_id: Option<AppId>,
    pub change_number: Option<u32>,
    pub kv_data: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub package_id: Option<PackageId>,
    pub change_number: Option<u32>,
    pub kv_data: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct BetaBranch {
    pub name: Option<String>,
    pub password: Option<String>,
    pub description: Option<String>,
}
