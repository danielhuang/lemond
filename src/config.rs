pub const CRITICAL_PROCESSES: &[(&str, Option<&[&str]>)] = &[
    ("/usr/bin/pipewire", None),
    ("/usr/bin/pipewire-pulse", None),
    ("/usr/bin/pipewire-media-session", None),
    ("/usr/bin/wireplumber", None),
    ("/usr/lib/systemd/systemd", None),
    ("/usr/bin/pulseaudio", None),
    ("/usr/bin/pulseeffects", None),
    ("/usr/bin/easyeffects", None),
    ("/usr/bin/kwin_wayland", None),
    (
        "/usr/bin/gnome-shell",
        Some(&["gnome-shel:cs0", "gnome-shell", "gnome-sh:gdrv0"]),
    ),
    ("/usr/bin/Xwayland", None),
    ("/usr/lib/Xorg", None),
    ("/usr/bin/gnome-system-monitor", None),
    ("/usr/bin/i3lock", None),
    ("/usr/lib/systemd/systemd-oomd", None),
    ("/usr/bin/ksysguard", None),
    ("/usr/lib/ksysguard/ksgrd_network_helper", None),
    ("/usr/bin/ksysguardd", None),
    ("/usr/bin/Hyprland", None),
    ("/usr/lib/mutter-x11-frames", None),
    ("/usr/bin/plasma-systemmonitor", None),
    ("/usr/bin/cosmic-comp", None),
];

pub const EXCLUDE_REALTIME: &[&str] = &[
    "/usr/bin/gnome-shell",
    "/usr/bin/Xwayland",
    "/usr/lib/mutter-x11-frames",
];

pub const GDB_MLOCK_PROCESSES: &[&str] = &[
    "/usr/lib/Xorg",
    "/usr/lib/systemd/systemd-oomd",
    "/usr/bin/pipewire",
    "/usr/bin/pipewire-pulse",
    "/usr/bin/pipewire-media-session",
    "/usr/bin/wireplumber",
    "/usr/bin/easyeffects",
    // "/usr/bin/gnome-shell",
    "/usr/bin/Xwayland",
    "/usr/lib/mutter-x11-frames",
    "/usr/bin/plasma-systemmonitor",
];

pub const LOW_PRIORITY_PROCESSES: &[&str] = &["rustc", "cc", "c++", "gcc", "g++", "makepkg", "cc1"];

pub const SCHED_IDLEPRIO: i32 = 5;
pub const SCHED_DEADLINE: i32 = 6;

pub const SOCKET_PATH: &str = "/run/lemond.socket";

pub const REALTIME_NICE: i32 = -20;
pub const BOOST_NICE: i32 = -7;
pub const LOW_PRIORITY_NICE: i32 = 19;

pub const ENABLE_REALTIME: bool = true;

pub const USE_THREAD_IDS: bool = true;

pub const FORCE_ASSIGN_NORMAL_SCHEDULER: bool = true;

pub const ENABLE_NICE: bool = true;

pub const ENABLE_SOCKET: bool = true;
pub const ENABLE_EXE_MLOCK: bool = false;
pub const ENABLE_LOCK_FDS: bool = false;
pub const ENABLE_GDB_MLOCK: bool = true;
pub const ENABLE_MEMORY_READ: bool = false;
pub const ENABLE_ZRAM_WRITEBACK: bool = false;

pub const ENABLE_KERNEL_TWEAKS: bool = true;

pub const LEMOND_SELF_REALTIME: bool = false;
