pub(crate) mod inner;

#[allow(clippy::needless_lifetimes)]
#[diplomat::bridge]
mod ffi {
    use diplomat_runtime::{DiplomatStr, DiplomatWrite};
    use std::fmt::Write;

    #[diplomat::opaque]
    pub struct SteamSession(pub(crate) crate::inner::SessionInner);

    #[diplomat::opaque]
    pub struct ManifestFileList(pub(crate) crate::inner::FileListInner);

    #[diplomat::opaque]
    pub struct SteamError(pub(crate) String);

    impl SteamSession {
        #[diplomat::attr(auto, constructor)]
        pub fn connect_anonymous() -> Result<Box<SteamSession>, Box<SteamError>> {
            crate::inner::connect_anonymous()
                .map(|s| Box::new(SteamSession(s)))
                .map_err(|e| Box::new(SteamError(e)))
        }

        pub fn connect_with_token(
            username: &DiplomatStr,
            token: &DiplomatStr,
        ) -> Result<Box<SteamSession>, Box<SteamError>> {
            let username = core::str::from_utf8(username)
                .map_err(|e| Box::new(SteamError(e.to_string())))?;
            let token = core::str::from_utf8(token)
                .map_err(|e| Box::new(SteamError(e.to_string())))?;
            crate::inner::connect_with_token(username, token)
                .map(|s| Box::new(SteamSession(s)))
                .map_err(|e| Box::new(SteamError(e)))
        }

        pub fn list_depot_files(
            &self,
            app_id: u32,
            depot_id: u32,
            branch: &DiplomatStr,
        ) -> Result<Box<ManifestFileList>, Box<SteamError>> {
            let branch = core::str::from_utf8(branch)
                .map_err(|e| Box::new(SteamError(e.to_string())))?;
            crate::inner::list_depot_files(&self.0, app_id, depot_id, branch)
                .map(|f| Box::new(ManifestFileList(f)))
                .map_err(|e| Box::new(SteamError(e)))
        }
    }

    impl ManifestFileList {
        pub fn len(&self) -> usize {
            self.0.names.len()
        }

        pub fn get_name(&self, index: usize, write: &mut DiplomatWrite) {
            if let Some(name) = self.0.names.get(index) {
                let _ = write!(write, "{}", name);
            }
        }

        pub fn get_size(&self, index: usize) -> u64 {
            self.0.sizes.get(index).copied().unwrap_or(0)
        }

        pub fn is_directory(&self, index: usize) -> bool {
            self.0.dirs.get(index).copied().unwrap_or(false)
        }
    }

    impl SteamError {
        pub fn message(&self, write: &mut DiplomatWrite) {
            let _ = write!(write, "{}", self.0);
        }
    }
}
