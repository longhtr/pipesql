//! Resolve query sources against a retained catalog or the fixed legacy schema.
//!
//! Storage identities describe persisted columns. Each occurrence receives its
//! own query identities, so a self-join has independent left and right values.
//! Read buffers and paths remain charged until released, before binding starts.
//! Source admission also checks the retained plan against the pinned schema.

use super::parser::{Parsed, ParsedStage};
use super::*;

/// Schemas and names for each source occurrence. A self-join reads the same
/// stored columns twice, but each side needs distinct query identities.
pub(super) struct SourceBindings {
    pub(super) outputs: [Output; MAX_COLUMNS],
    pub(super) columns: [SourceColumn; MAX_COLUMNS],
    pub(super) catalog: [u32; MAX_COLUMNS],
    pub(super) occurrences: [SourceOccurrence; MAX_STAGES],
    pub(super) count: u8,
}

impl SourceBindings {
    fn empty() -> Self {
        Self {
            outputs: [Output::EMPTY; MAX_COLUMNS],
            columns: [SourceColumn::QUANTITY; MAX_COLUMNS],
            catalog: [0; MAX_COLUMNS],
            occurrences: [SourceOccurrence::EMPTY; MAX_STAGES],
            count: 0,
        }
    }
}

pub(super) fn legacy(source: &str, parsed: &Parsed) -> Result<SourceBindings, Error> {
    for stage in &parsed.stages[..usize::from(parsed.len)] {
        match stage {
            ParsedStage::UnionAll(span) => {
                return Err(bind_error("UNION requires declared-table storage", *span));
            }
            ParsedStage::IntersectDistinct(span) | ParsedStage::IntersectAll(span) => {
                return Err(bind_error(
                    "INTERSECT requires declared-table storage",
                    *span,
                ));
            }
            ParsedStage::ExceptDistinct(span) | ParsedStage::ExceptAll(span) => {
                return Err(bind_error("EXCEPT requires declared-table storage", *span));
            }
            _ => (),
        }
    }
    if parsed.source_count != 1 {
        return Err(bind_error(
            "joins require declared-table storage",
            parsed.sources[1].table,
        ));
    }

    if let Some(item) = parsed.order_items[..usize::from(parsed.order_count)].first() {
        return Err(bind_error(
            "standalone ordering requires declared-table storage",
            item.reference,
        ));
    }
    let table = parsed.sources[0].table;
    if !text(source, table).eq_ignore_ascii_case("lineitem") {
        return Err(bind_error("unknown source table", table));
    }
    let (outputs, columns, count) = initial_outputs();
    let mut sources = SourceBindings::empty();
    sources.outputs = outputs;
    sources.columns = columns;
    sources.count = count;
    sources.occurrences[0] = SourceOccurrence {
        table: None,
        start: 0,
        columns: count,
    };
    Ok(sources)
}

pub(super) fn catalog(
    database: &Database,
    snapshot: &crate::storage::snapshot::Snapshot<'_>,
    source: &str,
    parsed: &Parsed,
) -> Result<SourceBindings, Error> {
    // Catalog and schema reads overlap a units pathname with one object pathname.
    // Scratch accounts only for its buffer. Admit both bounded paths before I/O,
    // and keep this charge until all read paths have been physically released.
    let paths = database.reserve_memory(
        (2 * crate::path::MAX_PATH_BYTES) as u64,
        "query catalog paths",
    )?;
    let mut scratch =
        crate::storage::catalog::Scratch::new(&database.memory, "query catalog binding")?;
    let (catalog_bytes, schema_bytes) = scratch
        .bytes()
        .split_at_mut(crate::storage::catalog::MAX_BYTES);
    let cancel = crate::CancellationToken::new();
    let mut effects = crate::effects::Effects::default();
    let catalog = snapshot
        .read_catalog(catalog_bytes, &cancel, &mut effects)?
        .ok_or_else(|| bind_error("unknown source table", parsed.table))?;
    let objects = crate::path::joined_path(database.path(), crate::storage::recovery::UNITS_NAME)?;
    let mut sources = SourceBindings::empty();
    for (occurrence, parsed_source) in parsed.sources[..usize::from(parsed.source_count)]
        .iter()
        .enumerate()
    {
        let ordinal = (0..catalog.len())
            .find(|&index| {
                catalog.table(index).is_some_and(|table| {
                    table
                        .name()
                        .eq_ignore_ascii_case(text(source, parsed_source.table))
                })
            })
            .ok_or_else(|| bind_error("unknown source table", parsed_source.table))?;
        let schema = catalog.read_schema(&objects, ordinal, schema_bytes, &cancel, &mut effects)?;
        let start = usize::from(sources.count);
        let end = start
            .checked_add(schema.len())
            .ok_or(Error::Corrupt("source width overflow"))?;
        if end > MAX_COLUMNS {
            return Err(bind_error(
                "query source column limit exceeded",
                parsed_source.table,
            ));
        }
        sources.occurrences[occurrence] = SourceOccurrence {
            table: Some(schema.table()),
            start: sources.count,
            columns: schema.len() as u8,
        };
        for index in 0..schema.len() {
            let column = schema.column(index).expect("validated schema ordinal");
            let slot = start + index;
            sources.catalog[slot] = column.id().value();
            let name = Name::new(column.name());
            if !name.valid() {
                return Err(bind_error(
                    "source column name is not admitted",
                    parsed_source.table,
                ));
            }
            sources.columns[slot] = SourceColumn::bind_declaration(column, index, slot as u32 + 1);
            sources.outputs[slot] = Output {
                name,
                id: sources.columns[slot].identity,
            };
        }
        sources.count = end as u8;
    }
    drop(objects);
    drop(scratch);
    drop(paths);
    Ok(sources)
}

impl SourceColumn {
    pub(super) fn bind_declaration(
        column: crate::storage::schema::ColumnSpec<'_>,
        ordinal: usize,
        identity: u32,
    ) -> Self {
        assert!(ordinal < MAX_COLUMNS);
        Self {
            identity: ColumnId::new(identity),
            storage: ordinal as u8,
            kind: column.data_type(),
            nullable: column.nullable(),
        }
    }
}

pub(crate) fn matches_source_declaration(
    plan: &Plan,
    occurrence: u8,
    source: SourceColumn,
    column: crate::storage::schema::ColumnSpec<'_>,
    ordinal: usize,
) -> bool {
    let Ok(binding) = plan.occurrence(occurrence) else {
        return false;
    };
    ordinal < usize::from(binding.columns)
        && plan.source_columns[usize::from(binding.start) + ordinal] == source
        && plan.catalog_columns[usize::from(binding.start) + ordinal] == column.id().value()
        && usize::from(source.storage) == ordinal
        && source.kind == column.data_type()
        && source.nullable == column.nullable()
}

#[cfg(test)]
pub(crate) fn matches_declaration(
    plan: &Plan,
    source: SourceColumn,
    column: crate::storage::schema::ColumnSpec<'_>,
    ordinal: usize,
) -> bool {
    plan.catalog_columns.get(ordinal).copied() == Some(column.id().value())
        && usize::from(source.storage) == ordinal
        && source.kind == column.data_type()
        && source.nullable == column.nullable()
}
