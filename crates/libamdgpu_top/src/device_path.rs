use anyhow::{anyhow, Context};
use crate::{
    AMDGPU::{
        self,
        DeviceHandle,
        GfxTargetVersion,
    },
    PCI,
};
use crate::stat::ProcInfo;
use std::path::PathBuf;
use std::fs;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DevicePath {
    pub render: PathBuf,
    pub card: PathBuf,
    pub pci: PCI::BUS_INFO,
    pub sysfs_path: PathBuf,
    pub device_id: Option<u32>,
    pub revision_id: Option<u32>,
    pub device_name: String,
    pub arc_proc_index: Arc<Mutex<Vec<ProcInfo>>>,
}

impl DevicePath {
    pub fn init(&self) -> anyhow::Result<DeviceHandle> {
        let (amdgpu_dev, _major, _minor) = {
            use std::os::unix::io::IntoRawFd;

            // need write option for GUI context
            // https://gitlab.freedesktop.org/mesa/mesa/-/issues/2424
            let f = fs::OpenOptions::new().read(true).write(true).open(&self.render)?;

            DeviceHandle::init(f.into_raw_fd())
                .map_err(|v| anyhow!(v))
                .context("Failed to DeviceHandle::init")?
        };

        Ok(amdgpu_dev)
    }

    pub fn get_device_path_list() -> Vec<Self> {
        let amdgpu_devices = fs::read_dir("/sys/bus/pci/drivers/amdgpu").unwrap_or_else(|_| {
            eprintln!("The AMDGPU driver is not loaded.");
            panic!();
        });

        amdgpu_devices.flat_map(|v| {
            let name = v.ok()?.file_name();

            /* 0000:00:00.0 */
            if name.len() < 12 { return None; }

            let pci = name.into_string().ok()?.parse::<PCI::BUS_INFO>().ok()?;

            Self::try_from(pci).ok()
        }).collect()
    }

    pub fn get_gfx_target_version_from_kfd(&self) -> Option<GfxTargetVersion> {
        let drm_render_minor = {
            const PRE: &str = "/dev/dri/renderD";
            const PRE_LEN: usize = PRE.len();
            let render = self.render.to_str()?;
            if !render.starts_with(PRE) { return None }

            format!("drm_render_minor {}", &render.get(PRE_LEN..)?)
        };

        let dirs = fs::read_dir("/sys/class/kfd/kfd/topology/nodes/").ok()?;
        let mut gfx_target_version = String::new();

        'node: for dir_entry in dirs.flatten() {
            let Ok(s) = fs::read_to_string(dir_entry.path().join("properties")) else {
                continue
            };
            let mut lines = s.lines();
            let Some(ver_str) = lines
                .find(|&l| l.starts_with("gfx_target_version")) else { continue };

            if lines.any(|l| l.starts_with(&drm_render_minor)) {
                gfx_target_version = ver_str.to_string();
                break 'node;
            }
        }

        const PRE_GFX_VER_LEN: usize = "gfx_target_version ".len();
        let gfx_target_version: u32 = gfx_target_version.get(PRE_GFX_VER_LEN..)?.parse().ok()?;

        Some(GfxTargetVersion::from(gfx_target_version))
    }

    pub fn check_if_device_is_active(&self) -> bool {
        let path = self.sysfs_path.join("power/runtime_status");
        let Ok(s) = std::fs::read_to_string(path) else { return false };

        s.starts_with("active")
    }

    pub fn menu_entry(&self) -> String {
        format!("{} ({})", self.device_name, self.pci)
    }
}

impl TryFrom<PCI::BUS_INFO> for DevicePath {
    type Error = std::io::Error;

    fn try_from(pci: PCI::BUS_INFO) -> Result<Self, Self::Error> {
        let render = pci.get_drm_render_path()?;
        let card = pci.get_drm_card_path()?;
        let sysfs_path = pci.get_sysfs_path();
        let [device_id, revision_id] = [pci.get_device_id(), pci.get_revision_id()];
        let device_name = if let [Some(did), Some(rid)] = [device_id, revision_id] {
            AMDGPU::find_device_name(did, rid)
                .unwrap_or(AMDGPU::DEFAULT_DEVICE_NAME.to_string())
        } else {
            String::new()
        };
        let arc_proc_index = Arc::new(Mutex::new(Vec::new()));

        Ok(Self {
            render,
            card,
            pci,
            sysfs_path,
            device_id,
            revision_id,
            device_name,
            arc_proc_index,
        })
    }
}

impl fmt::Debug for DevicePath {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("DevicePath")
            .field("render", &self.render)
            .field("card", &self.card)
            .field("pci", &self.pci.to_string())
            .field("sysfs_path", &self.sysfs_path)
            .field("device_id", &self.device_id)
            .field("revision_id", &self.revision_id)
            .field("device_name", &self.device_name)
            .finish()
    }
}
