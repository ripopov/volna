//! Hierarchy count labels for the viewer.

pub trait ScopeSizeLabel {
    fn label(&self) -> String;
    fn detail(&self) -> String;
}
impl ScopeSizeLabel for volna_trace::data::ScopeSize {
    fn label(&self) -> String {
        grouped(self.signals)
    }

    fn detail(&self) -> String {
        let n = |count: u32, one: &str| {
            format!(
                "{} {one}{}",
                grouped(count),
                if count == 1 { "" } else { "s" }
            )
        };
        format!(
            "{} · {} · {}",
            n(self.signals, "signal"),
            n(self.variables, "variable"),
            n(self.scopes, "scope")
        )
    }
}

pub(crate) fn grouped(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
