use rkyv::{Archive, Deserialize, Serialize};
use super::params::*;
use super::JobId;

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub enum Request {
    Download { args: DownloadParams, priority: bool },
    Info { args: InfoParams, priority: bool },
    Files { args: FilesParams, priority: bool },
    Manifests { args: ManifestsParams, priority: bool },
    Diff { args: DiffParams, priority: bool },
    Packages { args: PackagesParams, priority: bool },
    SaveManifest { args: SaveManifestParams, priority: bool },
    Workshop { args: WorkshopParams, priority: bool },
    LocalInfo { args: LocalInfoParams, priority: bool },

    Status,
    Subscribe,
    Attach { job_id: JobId },
    Cancel { job_id: JobId },
    TogglePriority { job_id: JobId },
    Stop { force: bool },
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::OutputFormat;
    use rkyv::rancor;

    #[test]
    fn info_request_round_trips() {
        let req = Request::Info {
            args: InfoParams { app: 480, format: Some(OutputFormat::Json), os: None, show_all: false },
            priority: true,
        };
        let bytes = rkyv::to_bytes::<rancor::Error>(&req).unwrap();
        let back = rkyv::from_bytes::<Request, rancor::Error>(&bytes).unwrap();
        match back {
            Request::Info { args, priority } => {
                assert_eq!(args.app, 480);
                assert!(priority);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn control_requests_round_trip() {
        for req in [Request::Status, Request::Subscribe, Request::Stop { force: true }] {
            let bytes = rkyv::to_bytes::<rancor::Error>(&req).unwrap();
            let _back = rkyv::from_bytes::<Request, rancor::Error>(&bytes).unwrap();
        }
    }
}
