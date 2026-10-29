//! Column identity, visibility and order for transaction and signal tables.

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionColumn {
    Label,
    Begin,
    Duration,
    End,
    Id,
    Status,
    Attributes,
    Attribute(String),
}

impl TransactionColumn {
    pub const ALL: [Self; 7] = [
        Self::Label,
        Self::Begin,
        Self::Duration,
        Self::End,
        Self::Id,
        Self::Status,
        Self::Attributes,
    ];
    pub const DEFAULT: [Self; 5] = [
        Self::Label,
        Self::Begin,
        Self::Duration,
        Self::Status,
        Self::Attributes,
    ];

    pub(super) fn canonical(self) -> Self {
        match self {
            Self::Attribute(key) if key == "vtr.label" => Self::Label,
            column => column,
        }
    }

    #[must_use]
    pub fn title(&self) -> &str {
        match self {
            Self::Label => "Label · vtr.label",
            Self::Begin => "Begin",
            Self::Duration => "Duration",
            Self::End => "End",
            Self::Id => "ID",
            Self::Status => "Status",
            Self::Attributes => "Attributes · preview",
            Self::Attribute(key) => key,
        }
    }
    #[must_use]
    pub fn width(&self) -> f32 {
        match self {
            Self::Label => 210.0,
            Self::Begin | Self::End => 120.0,
            Self::Duration => 105.0,
            Self::Id => 100.0,
            Self::Status => 90.0,
            Self::Attributes => 360.0,
            Self::Attribute(_) => 160.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransactionColumns {
    entries: Vec<(TransactionColumn, bool)>,
    order: Vec<usize>,
}

impl Default for TransactionColumns {
    fn default() -> Self {
        Self {
            entries: TransactionColumn::ALL
                .into_iter()
                .map(|column| {
                    let visible = TransactionColumn::DEFAULT.contains(&column);
                    (column, visible)
                })
                .collect(),
            order: (0..TransactionColumn::ALL.len()).collect(),
        }
    }
}

impl TransactionColumns {
    pub(super) fn logs() -> Self {
        let mut columns = Self::default();
        for (_, visible) in &mut columns.entries {
            *visible = false;
        }
        columns.toggle(TransactionColumn::Begin);
        columns.toggle(TransactionColumn::Attribute("log.severity".into()));
        columns.toggle(TransactionColumn::Attribute("log.message".into()));
        columns
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    #[must_use]
    pub fn column(&self, index: usize) -> Option<&TransactionColumn> {
        self.entries.get(index).map(|(column, _)| column)
    }
    #[must_use]
    pub fn visible(&self, column: &TransactionColumn) -> bool {
        self.entries
            .iter()
            .any(|(c, visible)| c == column && *visible)
    }
    pub fn iter(&self) -> impl Iterator<Item = (usize, &TransactionColumn, bool)> {
        self.order.iter().map(|&index| {
            let (column, visible) = &self.entries[index];
            (index, column, *visible)
        })
    }
    fn toggle(&mut self, column: TransactionColumn) -> bool {
        let column = column.canonical();
        if let Some(index) = self.entries.iter().position(|(c, _)| c == &column) {
            if self.entries[index].1 && self.entries.iter().filter(|(_, v)| *v).count() == 1 {
                return false;
            }
            self.entries[index].1 = !self.entries[index].1;
        } else {
            let index = self.entries.len();
            self.entries.push((column, true));
            let before = self
                .order
                .iter()
                .position(|&i| self.entries[i].0 == TransactionColumn::Attributes)
                .unwrap_or(self.order.len());
            self.order.insert(before, index);
        }
        true
    }
    pub fn move_before(&mut self, source: usize, before: Option<usize>) -> bool {
        if before == Some(source) || source >= self.len() || before.is_some_and(|i| i >= self.len())
        {
            return false;
        }
        self.order.retain(|&i| i != source);
        let position = before.map_or(self.order.len(), |before| {
            self.order
                .iter()
                .position(|&i| i == before)
                .expect("column order")
        });
        self.order.insert(position, source);
        true
    }
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<(TransactionColumn, bool)>()
            + self.order.capacity() * std::mem::size_of::<usize>()
            + self
                .entries
                .iter()
                .map(|(column, _)| match column {
                    TransactionColumn::Attribute(key) => key.capacity(),
                    _ => 0,
                })
                .sum::<usize>()
    }
    #[must_use]
    pub fn validate(&self) -> bool {
        let n = self.len();
        n >= TransactionColumn::ALL.len()
            && self.order.len() == n
            && (0..n).all(|i| self.order.iter().filter(|&&j| j == i).count() == 1)
            && self.entries.iter().any(|(_, visible)| *visible)
            && self.entries.iter().enumerate().all(|(i, (column, _))| {
                if i < TransactionColumn::ALL.len() {
                    column == &TransactionColumn::ALL[i]
                } else {
                    matches!(column, TransactionColumn::Attribute(key) if key != "vtr.label")
                        && !self.entries[..i].iter().any(|(c, _)| c == column)
                }
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColumnSet {
    Transactions(TransactionColumns),
    Signals { time: bool, visible: Vec<bool> },
}

impl ColumnSet {
    #[must_use]
    pub fn transactions_default() -> Self {
        Self::Transactions(TransactionColumns::default())
    }
    #[must_use]
    pub fn signals(count: usize) -> Self {
        Self::Signals {
            time: true,
            visible: vec![true; count],
        }
    }
    #[must_use]
    pub fn transaction_visible(&self, column: TransactionColumn) -> bool {
        matches!(self, Self::Transactions(columns) if columns.visible(&column.canonical()))
    }
    pub fn toggle_transaction(&mut self, column: TransactionColumn) -> bool {
        match self {
            Self::Transactions(columns) => columns.toggle(column),
            Self::Signals { .. } => false,
        }
    }
    pub fn move_transaction(&mut self, source: usize, before: Option<usize>) -> bool {
        match self {
            Self::Transactions(columns) => columns.move_before(source, before),
            Self::Signals { .. } => false,
        }
    }
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Transactions(columns) => columns.len(),
            Self::Signals { visible, .. } => visible.len() + 1,
        }
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[must_use]
    pub fn bytes(&self) -> usize {
        match self {
            Self::Transactions(columns) => columns.bytes(),
            Self::Signals { visible, .. } => visible.capacity(),
        }
    }
    pub fn toggle_signal(&mut self, index: usize) -> bool {
        let Self::Signals { time, visible } = self else {
            return false;
        };
        if index == 0 {
            if !visible.iter().any(|v| *v) {
                return false;
            }
            *time = !*time;
        } else if index - 1 < visible.len() {
            let only_data_column = !*time && visible.iter().filter(|v| **v).count() == 1;
            let value = &mut visible[index - 1];
            if *value && only_data_column {
                return false;
            }
            *value = !*value;
        } else {
            return false;
        }
        true
    }
    pub fn reset(&mut self) {
        match self {
            Self::Transactions(columns) => *columns = TransactionColumns::default(),
            Self::Signals { time, visible } => {
                *time = true;
                visible.fill(true);
            }
        }
    }
}
