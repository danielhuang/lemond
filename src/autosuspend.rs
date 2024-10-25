use serde::{Deserialize, Serialize};
use std::{collections::HashMap, process::Command};

#[derive(Serialize, Deserialize, Debug)]
pub struct DrmInfo {
    #[serde(flatten)]
    cards: HashMap<String, Card>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Card {
    pub driver: Driver,
    pub device: Device,
    pub fb_size: FramebufferSize,
    pub connectors: Vec<Connector>,
    pub encoders: Vec<Encoder>,
    pub crtcs: Vec<Crtc>,
    pub planes: Vec<Plane>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Driver {
    pub name: String,
    pub desc: String,
    pub version: Version,
    pub kernel: Kernel,
    #[serde(rename = "client_caps")]
    pub client_caps: ClientCaps,
    pub caps: Caps,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Version {
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
    pub date: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Kernel {
    pub sysname: String,
    pub release: String,
    pub version: String,
    pub tainted: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientCaps {
    #[serde(rename = "STEREO_3D")]
    pub stereo_3d: bool,
    #[serde(rename = "UNIVERSAL_PLANES")]
    pub universal_planes: bool,
    #[serde(rename = "ATOMIC")]
    pub atomic: bool,
    #[serde(rename = "ASPECT_RATIO")]
    pub aspect_ratio: bool,
    #[serde(rename = "WRITEBACK_CONNECTORS")]
    pub writeback_connectors: bool,
    #[serde(rename = "CURSOR_PLANE_HOTSPOT")]
    pub cursor_plane_hotspot: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Caps {
    #[serde(rename = "DUMB_BUFFER")]
    pub dumb_buffer: i32,
    #[serde(rename = "VBLANK_HIGH_CRTC")]
    pub vblank_high_crtc: i32,
    #[serde(rename = "DUMB_PREFERRED_DEPTH")]
    pub dumb_preferred_depth: i32,
    #[serde(rename = "DUMB_PREFER_SHADOW")]
    pub dumb_prefer_shadow: i32,
    #[serde(rename = "PRIME")]
    pub prime: i32,
    #[serde(rename = "TIMESTAMP_MONOTONIC")]
    pub timestamp_monotonic: i32,
    #[serde(rename = "ASYNC_PAGE_FLIP")]
    pub async_page_flip: i32,
    #[serde(rename = "CURSOR_WIDTH")]
    pub cursor_width: i32,
    #[serde(rename = "CURSOR_HEIGHT")]
    pub cursor_height: i32,
    #[serde(rename = "ADDFB2_MODIFIERS")]
    pub addfb2_modifiers: i32,
    #[serde(rename = "PAGE_FLIP_TARGET")]
    pub page_flip_target: i32,
    #[serde(rename = "CRTC_IN_VBLANK_EVENT")]
    pub crtc_in_vblank_event: i32,
    #[serde(rename = "SYNCOBJ")]
    pub syncobj: i32,
    #[serde(rename = "SYNCOBJ_TIMELINE")]
    pub syncobj_timeline: i32,
    #[serde(rename = "ATOMIC_ASYNC_PAGE_FLIP")]
    pub atomic_async_page_flip: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Device {
    pub available_nodes: i32,
    pub bus_type: i32,
    pub device_data: DeviceData,
    pub bus_data: BusData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceData {
    pub vendor: i32,
    pub device: i32,
    pub subsystem_vendor: i32,
    pub subsystem_device: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BusData {
    pub domain: i32,
    pub bus: i32,
    pub slot: i32,
    pub function: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FramebufferSize {
    pub min_width: i32,
    pub max_width: i32,
    pub min_height: i32,
    pub max_height: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Connector {
    pub id: i32,
    #[serde(rename = "type")]
    pub connector_type: i32,
    pub status: i32,
    pub phy_width: i32,
    pub phy_height: i32,
    pub subpixel: i32,
    pub encoder_id: i32,
    pub encoders: Vec<i32>,
    pub modes: Vec<Mode>,
    pub properties: HashMap<String, Property>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Mode {
    pub clock: i32,
    pub hdisplay: i32,
    pub hsync_start: i32,
    pub hsync_end: i32,
    pub htotal: i32,
    pub hskew: i32,
    pub vdisplay: i32,
    pub vsync_start: i32,
    pub vsync_end: i32,
    pub vtotal: i32,
    pub vscan: i32,
    pub vrefresh: i32,
    pub flags: i64,
    #[serde(rename = "type")]
    pub mode_type: i32,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Property {
    pub id: i32,
    pub flags: i64,
    #[serde(rename = "type")]
    pub property_type: i32,
    pub atomic: bool,
    pub immutable: bool,
    pub raw_value: i128,
    pub spec: Option<serde_json::Value>,
    pub value: Option<serde_json::Value>,
    pub data: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Encoder {
    pub id: i32,
    #[serde(rename = "type")]
    pub encoder_type: i32,
    pub crtc_id: i32,
    pub possible_crtcs: i32,
    pub possible_clones: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Crtc {
    pub id: i32,
    pub fb_id: i32,
    pub x: i32,
    pub y: i32,
    pub mode: Option<Mode>,
    pub gamma_size: i32,
    pub properties: HashMap<String, Property>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Plane {
    pub id: i32,
    pub possible_crtcs: i32,
    pub crtc_id: i32,
    pub fb_id: i32,
    pub crtc_x: i32,
    pub crtc_y: i32,
    pub x: i32,
    pub y: i32,
    pub gamma_size: i32,
    pub fb: Option<Framebuffer>,
    pub formats: Vec<i32>,
    pub properties: HashMap<String, Property>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Framebuffer {
    pub id: i32,
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub modifier: i64,
    pub planes: Vec<FbPlane>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FbPlane {
    pub offset: i32,
    pub pitch: i32,
}

pub fn run() {
    let output = Command::new("drm_info").arg("-j").output().unwrap().stdout;
    let output = String::from_utf8(output).unwrap();
    let output: HashMap<String, Card> = serde_json::from_str(&output).unwrap();

    let all_modes: Vec<_> = output
        .values()
        .flat_map(|x| x.crtcs.iter().flat_map(|x| &x.mode))
        .collect();

    if all_modes.is_empty() {
        println!("no displays found, suspending");

        Command::new("systemctl")
            .arg("suspend")
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
    }
}
