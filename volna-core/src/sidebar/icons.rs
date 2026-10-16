//! Semantic glyphs and tints.

use crate::icons::IconName;
use crate::theme::Theme;
use volna_trace::data::{Direction, Hierarchy, Member, ScopeView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    None,
    Pipeline,
    Log,
    Generator,
    In,
    Out,
    InOut,
}

impl Tint {
    pub fn color<C: Copy>(self, theme: &Theme<C>) -> C {
        match self {
            Self::None | Self::In | Self::Out | Self::InOut => theme.panel.icon_muted,
            Self::Pipeline => theme.sidebar_tints[0],
            Self::Log => theme.sidebar_tints[1],
            Self::Generator => theme.sidebar_tints[2],
        }
    }
}

#[must_use]
pub fn generator_icon(kind: &str) -> (IconName, Tint) {
    match kind {
        "PIPELINE" => (IconName::Workflow, Tint::Pipeline),
        "LOG" => (IconName::ScrollText, Tint::Log),
        _ => (IconName::ChartNoAxesGantt, Tint::Generator),
    }
}

#[must_use]
pub fn scope_icon(scope: &ScopeView<'_>) -> (IconName, Tint) {
    (scope_kind_icon(scope.kind), Tint::None)
}

#[must_use]
pub fn scope_kind_icon(kind: &str) -> IconName {
    use IconName::{
        Box, Boxes, Braces, Brackets, Cable, Component, Cpu, Folder, GitFork, Layers, Library,
        Package, Server, SquareFunction, Terminal, Zap,
    };
    match kind {
        "module" | "vhdl_architecture" => Box,
        "sc_module" => Boxes,
        "core" => Cpu,
        "struct" | "union" | "vhdl_record" => Braces,
        "class" => Component,
        "interface" => Cable,
        "package" | "vhdl_package" => Package,
        "task" | "function" | "vhdl_procedure" | "vhdl_function" => SquareFunction,
        "generate" | "vhdl_generate" | "vhdl_for_generate" | "vhdl_if_generate" => GitFork,
        "begin" | "fork" | "vhdl_block" => Brackets,
        "vhdl_process" => Zap,
        "sv_array" => Layers,
        "program" => Terminal,
        "resource" => Server,
        "instrumentation_scope" => Library,
        _ => Folder,
    }
}

#[must_use]
pub fn member_icon(h: &Hierarchy, member: Member) -> IconName {
    match member {
        Member::Var(id) => {
            let v = h.var(id);
            match v.var_type {
                "parameter" => IconName::Pi,
                _ if v.enum_table.is_some() => IconName::Tags,
                "integer" => IconName::Hash,
                _ => super::members::shape_icon(v.shape),
            }
        }
        Member::Generator(_) if h.is_log(member) => IconName::MessageSquare,
        Member::Generator(id) => generator_icon(&h.generators()[id].kind).0,
    }
}

#[must_use]
pub fn direction_icon(direction: Direction) -> Option<(IconName, Tint)> {
    match direction {
        Direction::Input => Some((IconName::LogIn, Tint::In)),
        Direction::Output => Some((IconName::LogOut, Tint::Out)),
        Direction::InOut => Some((IconName::ArrowLeftRight, Tint::InOut)),
        Direction::None => None,
    }
}
