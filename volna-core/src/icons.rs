//! The single icon set (Lucide) and the bundled fonts, compiled in so every frontend
//! renders identically.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum IconName {
    Boxes,
    Cpu,
    Component,
    Cable,
    Package,
    SquareFunction,
    GitFork,
    Brackets,
    Zap,
    Layers,
    Terminal,
    Server,
    Library,
    Workflow,
    ScrollText,
    ChartNoAxesGantt,
    Hash,
    Pi,
    Tags,
    MessageSquare,
    LogIn,
    LogOut,
    ArrowLeftRight,
    ArrowUp,
    ArrowDown,
    ArrowUpDown,
    Activity,
    AudioWaveform,
    Binary,
    Box,
    Braces,
    ChevronDown,
    ChevronRight,
    ChevronsLeft,
    ChevronsRight,
    CircleDot,
    Folder,
    FolderOpen,
    ListTree,
    ListFilter,
    LoaderCircle,
    Locate,
    PanelLeft,
    PanelRight,
    Pin,
    Plus,
    Search,
    Settings,
    Sigma,
    TableColumns,
    TableDetails,
    TableFirst,
    TableLast,
    TableNext,
    TablePrevious,
    TriangleAlert,
    Type,
    WindowClose,
    WindowMaximize,
    WindowMinimize,
    WindowRestore,
    X,
}

macro_rules! icons {
    ($($variant:ident => $name:literal),* $(,)?) => {
        impl IconName {
            pub const ALL: &'static [IconName] = &[$(IconName::$variant),*];

            pub fn path(self) -> &'static str {
                match self { $(IconName::$variant => concat!("icons/", $name, ".svg")),* }
            }

            pub fn svg(self) -> &'static [u8] {
                match self {
                    $(IconName::$variant => include_bytes!(concat!("../assets/icons/", $name, ".svg"))),*
                }
            }
        }
    };
}

icons!(
    Boxes => "boxes",
    Cpu => "cpu",
    Component => "component",
    Cable => "cable",
    Package => "package",
    SquareFunction => "square-function",
    GitFork => "git-fork",
    Brackets => "brackets",
    Zap => "zap",
    Layers => "layers",
    Terminal => "terminal",
    Server => "server",
    Library => "library",
    Workflow => "workflow",
    ScrollText => "scroll-text",
    ChartNoAxesGantt => "chart-no-axes-gantt",
    Hash => "hash",
    Pi => "pi",
    Tags => "tags",
    MessageSquare => "message-square",
    LogIn => "log-in",
    LogOut => "log-out",
    ArrowLeftRight => "arrow-left-right",
    ArrowUp => "arrow-up",
    ArrowDown => "arrow-down",
    ArrowUpDown => "arrow-up-down",
    Activity => "activity",
    AudioWaveform => "audio-waveform",
    Binary => "binary",
    Box => "box",
    Braces => "braces",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    ChevronsLeft => "chevrons-left",
    ChevronsRight => "chevrons-right",
    CircleDot => "circle-dot",
    Folder => "folder",
    FolderOpen => "folder-open",
    ListTree => "list-tree",
    ListFilter => "list-filter",
    LoaderCircle => "loader-circle",
    Locate => "locate",
    PanelLeft => "panel-left",
    PanelRight => "panel-right",
    Pin => "pin",
    Plus => "plus",
    Search => "search",
    Settings => "settings",
    Sigma => "sigma",
    TableColumns => "table-columns",
    TableDetails => "table-details",
    TableFirst => "table-first",
    TableLast => "table-last",
    TableNext => "table-next",
    TablePrevious => "table-previous",
    TriangleAlert => "triangle-alert",
    Type => "type",
    WindowClose => "window-close",
    WindowMaximize => "window-maximize",
    WindowMinimize => "window-minimize",
    WindowRestore => "window-restore",
    X => "x",
);

impl IconName {
    #[must_use]
    pub fn from_path(path: &str) -> Option<IconName> {
        IconName::ALL.iter().copied().find(|i| i.path() == path)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FontFace {
    pub family: &'static str,
    pub weight: u16,
    pub bytes: &'static [u8],
}

pub const FONTS: &[FontFace] = &[
    FontFace {
        family: "Inter",
        weight: 400,
        bytes: include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    },
    FontFace {
        family: "Inter",
        weight: 500,
        bytes: include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    },
    FontFace {
        family: "Inter",
        weight: 600,
        bytes: include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    },
    FontFace {
        family: "JetBrains Mono",
        weight: 400,
        bytes: include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    },
    FontFace {
        family: "IBM Plex Sans",
        weight: 400,
        bytes: include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    },
];
