//! Persistent identities, root/fence authority, and the fixed-schema unit codec.
//! Codec functions only inspect or construct bytes; namespace/publication owners
//! decide when those bytes may be read, repaired, or made durable.

#[cfg(test)]
use std::array;

const FORMAT_VERSION: u32 = 4;
pub(crate) const CATALOG_FORMAT_VERSION: u32 = 7;
pub(super) const CONTROL_BYTES: usize = 128;
pub(super) const ROOT_BYTES: usize = 4_096;
pub(super) const WAL_BYTES: usize = 512;
pub(super) const HEADER_BYTES: usize = 4_096;
pub(super) const DESCRIPTOR_CAPACITY: usize = 1_344;
pub(super) const DESCRIPTOR_BYTES: usize = DESCRIPTOR_CAPACITY * 16;
pub(super) const UNIT_PADDING_BYTES: usize = 3_072;
pub(super) const PAYLOAD_OFFSET: u64 = 28_672;
pub(crate) const MAX_ROWS: u64 = 6_500_000;
const DOUBLE_ROWS_PER_BLOCK: u64 = 32_768;
const DATE_ROWS_PER_BLOCK: u64 = 65_536;
const BLOCK_BYTES: u64 = 262_144;

const CONTROL_MAGIC: &[u8; 8] = b"PIPESQL\0";
const ROOT_MAGIC: &[u8; 8] = b"PSQLROOT";
const WAL_MAGIC: &[u8; 8] = b"PSQLWAL\0";
const UNIT_MAGIC: &[u8; 8] = b"PSQLUNIT";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FormatError {
    Length,
    Magic,
    Version,
    Reserved,
    Checksum,
    Identity,
    Generation(u64),
    Field,
    Overflow,
}

/// The nonzero, persistent identity assigned when a database is created.
///
/// Reopening preserves this identity. Paths and generation numbers are separate
/// facts; use [`crate::Database::database_identity`] to obtain the current identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseId([u8; 16]);

impl DatabaseId {
    pub(super) fn new(bytes: [u8; 16]) -> Result<Self, FormatError> {
        if bytes == [0; 16] {
            return Err(FormatError::Identity);
        }
        Ok(Self(bytes))
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, FormatError> {
        let bytes: [u8; 16] = bytes.try_into().map_err(|_| FormatError::Length)?;
        Self::new(bytes)
    }

    /// Borrow the database identity's 16 bytes without allocation.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// An issued transaction attempt's identity, independent of its outcome.
///
/// The 24-byte token contains the database identity followed by a nonzero
/// little-endian attempt sequence. Keep it to resolve an uncertain commit with
/// [`crate::Database::resolve_commit`] after recovery. Decoding a token does not
/// establish that the attempt was issued or committed.
///
/// [`Display`](std::fmt::Display) renders 48 lowercase hexadecimal digits for
/// the CLI's `resolve --transaction` argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionId([u8; 24]);

impl TransactionId {
    pub(super) fn for_attempt(database: DatabaseId, sequence: u64) -> Result<Self, FormatError> {
        if sequence == 0 {
            return Err(FormatError::Identity);
        }
        let mut bytes = [0; 24];
        bytes[..16].copy_from_slice(database.as_bytes());
        bytes[16..].copy_from_slice(&sequence.to_le_bytes());
        Ok(Self(bytes))
    }

    pub(super) fn decode(bytes: [u8; 24]) -> Result<Self, FormatError> {
        DatabaseId::from_slice(&bytes[..16])?;
        let value = Self(bytes);
        if value.sequence() == 0 {
            return Err(FormatError::Identity);
        }
        Ok(value)
    }

    pub(super) fn sequence(self) -> u64 {
        u64::from_le_bytes(self.0[16..].try_into().expect("fixed attempt sequence"))
    }

    pub(super) fn belongs_to(self, database: DatabaseId) -> bool {
        self.0[..16] == database.0
    }

    /// Borrow the token bytes accepted by [`Self::from_bytes`].
    pub fn as_bytes(&self) -> &[u8; 24] {
        &self.0
    }
}

impl std::fmt::Display for TransactionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Replica {
    A,
    B,
}

impl Replica {
    fn byte(self) -> u8 {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }

    fn from_byte(byte: u8) -> Result<Self, FormatError> {
        match byte {
            0 => Ok(Self::A),
            1 => Ok(Self::B),
            _ => Err(FormatError::Field),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Layout {
    pub(super) rows: u64,
    pub(super) double_blocks: u32,
    pub(super) date_blocks: u32,
    pub(super) descriptor_count: u32,
    pub(super) payload_bytes: u64,
    pub(super) unit_bytes: u64,
    pub(super) temporary_peak_bytes: u64,
}

pub(super) fn layout(rows: u64) -> Result<Layout, FormatError> {
    if rows > MAX_ROWS {
        return Err(FormatError::Field);
    }
    let double_blocks = ceiling_div(rows, DOUBLE_ROWS_PER_BLOCK)?;
    let date_blocks = ceiling_div(rows, DATE_ROWS_PER_BLOCK)?;
    let descriptor_count = double_blocks
        .checked_mul(6)
        .and_then(|value| value.checked_add(date_blocks))
        .ok_or(FormatError::Overflow)?;
    if descriptor_count > DESCRIPTOR_CAPACITY as u64 {
        return Err(FormatError::Field);
    }
    let payload_bytes = rows.checked_mul(38).ok_or(FormatError::Overflow)?;
    let unit_bytes = PAYLOAD_OFFSET
        .checked_add(payload_bytes)
        .ok_or(FormatError::Overflow)?;
    let temporary_peak_bytes = payload_bytes
        .checked_add(unit_bytes)
        .ok_or(FormatError::Overflow)?;
    Ok(Layout {
        rows,
        double_blocks: u32::try_from(double_blocks).map_err(|_| FormatError::Overflow)?,
        date_blocks: u32::try_from(date_blocks).map_err(|_| FormatError::Overflow)?,
        descriptor_count: u32::try_from(descriptor_count).map_err(|_| FormatError::Overflow)?,
        payload_bytes,
        unit_bytes,
        temporary_peak_bytes,
    })
}

fn ceiling_div(value: u64, divisor: u64) -> Result<u64, FormatError> {
    if value == 0 {
        return Ok(0);
    }
    value
        .checked_sub(1)
        .and_then(|value| value.checked_div(divisor))
        .and_then(|value| value.checked_add(1))
        .ok_or(FormatError::Overflow)
}

// Declared-table graph state; legacy roots retain their fixed-unit representation.
// Private fields exclude partial receipts and reference/count disagreement.
pub(super) const MAX_SUCCESSES: u64 = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CatalogCommit {
    generation: u64,
    transaction: TransactionId,
    catalog: crate::catalog::ObjectRef,
    successes: crate::catalog::ObjectRef,
}

impl CatalogCommit {
    pub(super) fn new(
        database: DatabaseId,
        generation: u64,
        transaction: TransactionId,
        catalog: crate::catalog::ObjectRef,
        successes: crate::catalog::ObjectRef,
    ) -> Result<Self, FormatError> {
        let sequence = transaction.sequence();
        if generation == 0
            || generation > MAX_SUCCESSES
            || generation > sequence
            || !transaction.belongs_to(database)
            || catalog.object().attempt() > sequence
            || successes.object().attempt() != sequence
            || catalog.object() == successes.object()
            || u64::from(catalog.bytes()) > crate::catalog::MAX_BYTES as u64
        {
            return Err(FormatError::Field);
        }
        let expected = generation
            .checked_mul(8)
            .and_then(|n| n.checked_add(64))
            .ok_or(FormatError::Overflow)?;
        if u64::from(successes.bytes()) != expected {
            return Err(FormatError::Length);
        }
        Ok(Self {
            generation,
            transaction,
            catalog,
            successes,
        })
    }

    pub(super) fn generation(self) -> u64 {
        self.generation
    }

    pub(super) fn transaction(self) -> TransactionId {
        self.transaction
    }

    pub(super) fn catalog(self) -> crate::catalog::ObjectRef {
        self.catalog
    }

    pub(super) fn successes(self) -> crate::catalog::ObjectRef {
        self.successes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RootState {
    Catalog(Option<CatalogCommit>),
    Empty,
    Data {
        transaction: TransactionId,
        rows: u64,
        unit_bytes: u64,
        unit_metadata_crc32c: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Root {
    pub(super) database: DatabaseId,
    pub(super) issued: u64,
    pub(super) replica: Replica,
    pub(super) state: RootState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WalRecord {
    pub(super) database: DatabaseId,
    pub(super) issued: u64,
    pub(super) state: RootState,
}

impl Root {
    pub(super) fn fence(self) -> WalRecord {
        WalRecord {
            database: self.database,
            issued: self.issued,
            state: self.state,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PublicationKind {
    Issuance,
    Commit,
}

impl WalRecord {
    pub(super) fn transition_from(self, old: Self) -> Option<PublicationKind> {
        if self.database != old.database {
            return None;
        }
        let issuance = old.issued.checked_add(1) == Some(self.issued) && self.state == old.state;
        let commit = self.issued == old.issued
            && match (old.state, self.state) {
                (RootState::Empty, RootState::Data { transaction, .. }) => {
                    transaction.sequence() == self.issued
                }
                (RootState::Catalog(prior), RootState::Catalog(Some(next))) => {
                    prior.map_or(0, CatalogCommit::generation).checked_add(1)
                        == Some(next.generation)
                        && next.transaction.sequence() == self.issued
                        && prior.is_none_or(|prior| prior.transaction.sequence() < self.issued)
                }
                _ => false,
            };
        if issuance {
            Some(PublicationKind::Issuance)
        } else if commit {
            Some(PublicationKind::Commit)
        } else {
            None
        }
    }

    pub(super) fn succeeds(self, old: Self) -> bool {
        self.transition_from(old).is_some()
    }
}

pub(super) fn select_snapshot(
    a: Option<Root>,
    b: Option<Root>,
    fence: Option<WalRecord>,
) -> Result<(WalRecord, Option<Replica>), FormatError> {
    let (selected, repair) = match (a, b) {
        (Some(a), Some(b)) if a.fence() == b.fence() => (a.fence(), None),
        (Some(a), Some(b)) if a.fence().succeeds(b.fence()) => (a.fence(), Some(Replica::B)),
        (Some(a), Some(b)) if b.fence().succeeds(a.fence()) => (b.fence(), Some(Replica::A)),
        (Some(a), None) if fence == Some(a.fence()) => (a.fence(), Some(Replica::B)),
        (None, Some(b)) if fence == Some(b.fence()) => (b.fence(), Some(Replica::A)),
        _ => return Err(FormatError::Field),
    };
    if let Some(fence) = fence
        && fence != selected
        && !fence.succeeds(selected)
    {
        return Err(FormatError::Field);
    }
    Ok((selected, repair))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockDescriptor {
    pub(super) offset: u64,
    pub(super) bytes: u32,
    pub(super) crc32c: u32,
}

impl BlockDescriptor {
    pub(super) const EMPTY: Self = Self {
        offset: 0,
        bytes: 0,
        crc32c: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UnitMetadata {
    pub(super) database: DatabaseId,
    pub(super) rows: u64,
    pub(super) projected_crc32c: u32,
    pub(super) descriptors: [BlockDescriptor; DESCRIPTOR_CAPACITY],
}

impl UnitMetadata {
    #[cfg(test)]
    pub(super) fn new(
        database: DatabaseId,
        rows: u64,
        projected_crc32c: u32,
        used: &[BlockDescriptor],
    ) -> Result<Self, FormatError> {
        let expected = layout(rows)?;
        if used.len() != expected.descriptor_count as usize {
            return Err(FormatError::Field);
        }
        let mut descriptors = [BlockDescriptor::EMPTY; DESCRIPTOR_CAPACITY];
        descriptors[..used.len()].copy_from_slice(used);
        let metadata = Self {
            database,
            rows,
            projected_crc32c,
            descriptors,
        };
        validate_descriptors(metadata.rows, Descriptors::Decoded(&metadata.descriptors))?;
        Ok(metadata)
    }

    pub(super) fn descriptor_count(&self) -> usize {
        layout(self.rows)
            .expect("validated unit row count")
            .descriptor_count as usize
    }
}

pub(super) fn encode_control(database: DatabaseId) -> [u8; CONTROL_BYTES] {
    encode_control_version(database, FORMAT_VERSION)
}

pub(super) fn encode_catalog_control(database: DatabaseId) -> [u8; CONTROL_BYTES] {
    encode_control_version(database, CATALOG_FORMAT_VERSION)
}

fn encode_control_version(database: DatabaseId, version: u32) -> [u8; CONTROL_BYTES] {
    let mut bytes = [0; CONTROL_BYTES];
    bytes[..8].copy_from_slice(CONTROL_MAGIC);
    put_u32(&mut bytes, 8, version);
    put_u32(&mut bytes, 12, CONTROL_BYTES as u32);
    bytes[16..32].copy_from_slice(&database.0);
    let checksum = crc32c(&bytes);
    put_u32(&mut bytes, 36, checksum);
    bytes
}

pub(super) fn decode_control(bytes: &[u8]) -> Result<DatabaseId, FormatError> {
    require_length(bytes, CONTROL_BYTES)?;
    if &bytes[..8] != CONTROL_MAGIC {
        return Err(FormatError::Magic);
    }
    let catalog = read_u32(bytes, 8)? == CATALOG_FORMAT_VERSION;
    if catalog {
        if read_u32(bytes, 12)? != CONTROL_BYTES as u32 {
            return Err(FormatError::Length);
        }
    } else {
        require_version_length(bytes, CONTROL_BYTES)?;
    }
    require_zero(&bytes[32..36])?;
    require_zero(&bytes[40..])?;
    require_checksum(bytes, 36)?;
    DatabaseId::from_slice(&bytes[16..32])
}

pub(super) fn encode_root(root: Root) -> Result<[u8; ROOT_BYTES], FormatError> {
    let mut bytes = [0; ROOT_BYTES];
    encode_snapshot(root, &mut bytes, ROOT_MAGIC)?;
    Ok(bytes)
}

fn encode_snapshot(root: Root, bytes: &mut [u8], magic: &[u8; 8]) -> Result<(), FormatError> {
    if let RootState::Catalog(committed) = root.state {
        return encode_catalog_snapshot(root, committed, bytes, magic);
    }
    bytes[..8].copy_from_slice(magic);
    put_u32(bytes, 8, FORMAT_VERSION);
    put_u32(
        bytes,
        12,
        u32::try_from(bytes.len()).map_err(|_| FormatError::Length)?,
    );
    bytes[16..32].copy_from_slice(&root.database.0);
    bytes[32] = root.replica.byte();
    put_u64(bytes, 112, root.issued);
    match root.state {
        RootState::Catalog(_) => unreachable!("catalog encoding dispatched above"),
        RootState::Empty => {}
        RootState::Data {
            transaction,
            rows,
            unit_bytes,
            unit_metadata_crc32c,
        } => {
            validate_data_fields(root.database, root.issued, transaction, rows, unit_bytes)?;
            put_u64(bytes, 40, 1);
            bytes[56..80].copy_from_slice(&transaction.0);
            put_u64(bytes, 80, 1);
            put_u64(bytes, 88, rows);
            put_u64(bytes, 96, unit_bytes);
            put_u32(bytes, 104, unit_metadata_crc32c);
        }
    }
    let checksum = crc32c(bytes);
    put_u32(bytes, 108, checksum);
    Ok(())
}

pub(super) fn decode_root(bytes: &[u8]) -> Result<Root, FormatError> {
    decode_snapshot(bytes, ROOT_BYTES, ROOT_MAGIC)
}

fn decode_snapshot(bytes: &[u8], length: usize, magic: &[u8; 8]) -> Result<Root, FormatError> {
    if bytes.len() >= 12 && &bytes[..8] == magic && read_u32(bytes, 8)? == CATALOG_FORMAT_VERSION {
        return decode_catalog_snapshot(bytes, length, magic);
    }
    // Version is recognizable from this bounded prefix even if a future format
    // has a different extent or a later write was interrupted. Such a copy is
    // unsupported authority, not permission to repair it as current-format damage.
    if bytes.len() >= 12 && &bytes[..8] == magic && read_u32(bytes, 8)? != FORMAT_VERSION {
        return Err(FormatError::Version);
    }
    require_length(bytes, length)?;
    if &bytes[..8] != magic {
        return Err(FormatError::Magic);
    }
    // Check integrity before interpreting semantic failures as unknown authority.
    // A checksum-valid unsupported field must not be repaired as random damage.
    require_checksum(bytes, 108)?;
    let generation = read_u64(bytes, 40)?;
    if generation > 1 {
        return Err(FormatError::Generation(generation));
    }
    if read_u32(bytes, 12)? != u32::try_from(length).map_err(|_| FormatError::Length)? {
        return Err(FormatError::Field);
    }
    require_zero(&bytes[33..40])?;
    require_zero(&bytes[48..56])?;
    require_zero(&bytes[120..])?;
    let database = DatabaseId::from_slice(&bytes[16..32])?;
    let replica = Replica::from_byte(bytes[32])?;
    let issued = read_u64(bytes, 112)?;
    let state = match generation {
        0 => {
            require_zero(&bytes[48..108])?;
            RootState::Empty
        }
        1 => {
            if read_u64(bytes, 80)? != 1 {
                return Err(FormatError::Field);
            }
            let transaction =
                TransactionId(bytes[56..80].try_into().map_err(|_| FormatError::Length)?);
            let rows = read_u64(bytes, 88)?;
            let unit_bytes = read_u64(bytes, 96)?;
            validate_data_fields(database, issued, transaction, rows, unit_bytes)?;
            RootState::Data {
                transaction,
                rows,
                unit_bytes,
                unit_metadata_crc32c: read_u32(bytes, 104)?,
            }
        }
        generation => return Err(FormatError::Generation(generation)),
    };
    Ok(Root {
        database,
        issued,
        replica,
        state,
    })
}

fn encode_catalog_snapshot(
    root: Root,
    committed: Option<CatalogCommit>,
    bytes: &mut [u8],
    magic: &[u8; 8],
) -> Result<(), FormatError> {
    bytes[..8].copy_from_slice(magic);
    put_u32(bytes, 8, CATALOG_FORMAT_VERSION);
    put_u32(
        bytes,
        12,
        u32::try_from(bytes.len()).map_err(|_| FormatError::Length)?,
    );
    bytes[16..32].copy_from_slice(root.database.as_bytes());
    bytes[32] = root.replica.byte();
    put_u64(bytes, 112, root.issued);
    if let Some(committed) = committed {
        if !committed.transaction.belongs_to(root.database)
            || committed.transaction.sequence() > root.issued
        {
            return Err(FormatError::Identity);
        }
        put_u64(bytes, 40, committed.generation);
        bytes[56..80].copy_from_slice(committed.transaction.as_bytes());
        committed.catalog.encode(
            (&mut bytes[128..152])
                .try_into()
                .expect("catalog reference field"),
        );
        committed.successes.encode(
            (&mut bytes[152..176])
                .try_into()
                .expect("success reference field"),
        );
    }
    put_u32(bytes, 108, crc32c(bytes));
    Ok(())
}

fn decode_catalog_snapshot(
    bytes: &[u8],
    length: usize,
    magic: &[u8; 8],
) -> Result<Root, FormatError> {
    require_length(bytes, length)?;
    if &bytes[..8] != magic {
        return Err(FormatError::Magic);
    }
    require_checksum(bytes, 108)?;
    if read_u32(bytes, 12)? != u32::try_from(length).map_err(|_| FormatError::Length)? {
        return Err(FormatError::Length);
    }
    require_zero(&bytes[33..40])?;
    require_zero(&bytes[48..56])?;
    require_zero(&bytes[80..108])?;
    require_zero(&bytes[120..128])?;
    require_zero(&bytes[176..])?;
    let database = DatabaseId::from_slice(&bytes[16..32])?;
    let replica = Replica::from_byte(bytes[32])?;
    let issued = read_u64(bytes, 112)?;
    let generation = read_u64(bytes, 40)?;
    let committed = if generation == 0 {
        require_zero(&bytes[56..80])?;
        require_zero(&bytes[128..176])?;
        None
    } else {
        let transaction =
            TransactionId::decode(bytes[56..80].try_into().expect("transaction field"))?;
        let catalog =
            crate::catalog::ObjectRef::decode(bytes[128..152].try_into().expect("catalog field"))?;
        let successes =
            crate::catalog::ObjectRef::decode(bytes[152..176].try_into().expect("success field"))?;
        if transaction.sequence() > issued {
            return Err(FormatError::Identity);
        }
        Some(CatalogCommit::new(
            database,
            generation,
            transaction,
            catalog,
            successes,
        )?)
    };
    Ok(Root {
        database,
        issued,
        replica,
        state: RootState::Catalog(committed),
    })
}

pub(super) fn encode_wal(record: WalRecord) -> Result<[u8; WAL_BYTES], FormatError> {
    let mut bytes = [0; WAL_BYTES];
    encode_snapshot(
        Root {
            database: record.database,
            issued: record.issued,
            replica: Replica::A,
            state: record.state,
        },
        &mut bytes,
        WAL_MAGIC,
    )?;
    Ok(bytes)
}

pub(super) fn decode_wal(bytes: &[u8]) -> Result<WalRecord, FormatError> {
    let root = decode_snapshot(bytes, WAL_BYTES, WAL_MAGIC)?;
    if root.replica != Replica::A {
        return Err(FormatError::Field);
    }
    Ok(root.fence())
}

fn validate_data_fields(
    database: DatabaseId,
    issued: u64,
    transaction: TransactionId,
    rows: u64,
    unit_bytes: u64,
) -> Result<(), FormatError> {
    if !transaction.belongs_to(database)
        || transaction.sequence() == 0
        || transaction.sequence() > issued
        || layout(rows)?.unit_bytes != unit_bytes
    {
        return Err(FormatError::Field);
    }
    Ok(())
}

pub(super) fn encode_unit_metadata_into(
    database: DatabaseId,
    rows: u64,
    projected_crc32c: u32,
    descriptors: &[BlockDescriptor; DESCRIPTOR_CAPACITY],
    header: &mut [u8; HEADER_BYTES],
    descriptor_bytes: &mut [u8; DESCRIPTOR_BYTES],
) -> Result<u32, FormatError> {
    validate_descriptors(rows, Descriptors::Decoded(descriptors))?;
    let geometry = layout(rows)?;
    header.fill(0);
    descriptor_bytes.fill(0);
    header[..8].copy_from_slice(UNIT_MAGIC);
    put_u32(header, 8, FORMAT_VERSION);
    put_u32(header, 12, HEADER_BYTES as u32);
    header[16..32].copy_from_slice(&database.0);
    put_u64(header, 32, 1);
    put_u64(header, 40, 1);
    put_u64(header, 48, rows);
    put_u64(header, 56, geometry.unit_bytes);
    put_u64(header, 64, HEADER_BYTES as u64);
    put_u32(header, 72, geometry.descriptor_count);
    put_u32(header, 76, DESCRIPTOR_CAPACITY as u32);
    put_u64(header, 80, DESCRIPTOR_BYTES as u64);
    put_u64(header, 88, PAYLOAD_OFFSET);
    put_u32(header, 96, 7);
    put_u32(header, 112, projected_crc32c);
    let columns = column_geometry(rows)?;
    for (index, column) in columns.into_iter().enumerate() {
        let base = 128 + index * 40;
        put_u32(header, base, column.id);
        put_u32(header, base + 4, column.kind);
        put_u32(header, base + 8, column.width);
        put_u32(header, base + 12, column.rows_per_block);
        put_u32(header, base + 16, column.first_descriptor);
        put_u32(header, base + 20, column.descriptor_count);
        put_u64(header, base + 24, column.offset);
        put_u64(header, base + 32, column.bytes);
    }
    for (index, descriptor) in descriptors
        [..usize::try_from(geometry.descriptor_count).map_err(|_| FormatError::Overflow)?]
        .iter()
        .enumerate()
    {
        let base = index * 16;
        put_u64(descriptor_bytes, base, descriptor.offset);
        put_u32(descriptor_bytes, base + 8, descriptor.bytes);
        put_u32(descriptor_bytes, base + 12, descriptor.crc32c);
    }
    let descriptor_crc = crc32c(descriptor_bytes);
    let metadata_crc = metadata_crc32c(header, descriptor_bytes);
    put_u32(header, 104, descriptor_crc);
    put_u32(header, 108, metadata_crc);
    let header_crc = crc32c(header);
    put_u32(header, 100, header_crc);
    Ok(metadata_crc)
}

#[cfg(test)]
pub(super) fn encode_unit_metadata(
    metadata: &UnitMetadata,
) -> Result<([u8; HEADER_BYTES], [u8; DESCRIPTOR_BYTES], u32), FormatError> {
    let mut header = [0; HEADER_BYTES];
    let mut descriptors = [0; DESCRIPTOR_BYTES];
    let checksum = encode_unit_metadata_into(
        metadata.database,
        metadata.rows,
        metadata.projected_crc32c,
        &metadata.descriptors,
        &mut header,
        &mut descriptors,
    )?;
    Ok((header, descriptors, checksum))
}

// Caller-owned scratch avoids moving a descriptor array through a large Result.
// No reference escapes on failure, and partially initialized scratch needs no
// destructor. Inputs can be reused once this constructor returns.
const _: () = assert!(!std::mem::needs_drop::<UnitMetadata>());

// Lifecycle inspection needs the validated scalar facts, not another 21-KiB
// descriptor owner. Both public decode forms use this same complete validator.
pub(super) struct UnitSummary {
    database: DatabaseId,
    rows: u64,
    projected_crc32c: u32,
}

impl UnitSummary {
    pub(super) fn database(&self) -> DatabaseId {
        self.database
    }

    pub(super) fn rows(&self) -> u64 {
        self.rows
    }

    pub(super) fn projected_crc32c(&self) -> u32 {
        self.projected_crc32c
    }
}

pub(super) fn decode_unit_summary(
    header: &[u8],
    descriptor_bytes: &[u8],
) -> Result<(UnitSummary, u32), FormatError> {
    require_length(header, HEADER_BYTES)?;
    require_length(descriptor_bytes, DESCRIPTOR_BYTES)?;
    if &header[..8] != UNIT_MAGIC {
        return Err(FormatError::Magic);
    }
    require_version_length(header, HEADER_BYTES)?;
    let generation = read_u64(header, 32)?;
    if generation != 1 {
        return Err(FormatError::Generation(generation));
    }
    if read_u64(header, 40)? != 1
        || read_u64(header, 64)? != HEADER_BYTES as u64
        || read_u32(header, 76)? != DESCRIPTOR_CAPACITY as u32
        || read_u64(header, 80)? != DESCRIPTOR_BYTES as u64
        || read_u64(header, 88)? != PAYLOAD_OFFSET
        || read_u32(header, 96)? != 7
    {
        return Err(FormatError::Field);
    }
    require_zero(&header[116..128])?;
    require_zero(&header[408..])?;
    require_checksum(header, 100)?;
    if read_u32(header, 104)? != crc32c(descriptor_bytes) {
        return Err(FormatError::Checksum);
    }
    let metadata_crc = metadata_crc32c(header, descriptor_bytes);
    if read_u32(header, 108)? != metadata_crc {
        return Err(FormatError::Checksum);
    }
    let database = DatabaseId::from_slice(&header[16..32])?;
    let rows = read_u64(header, 48)?;
    let geometry = layout(rows)?;
    if read_u64(header, 56)? != geometry.unit_bytes
        || read_u32(header, 72)? != geometry.descriptor_count
    {
        return Err(FormatError::Field);
    }
    let expected_columns = column_geometry(rows)?;
    for (index, expected) in expected_columns.into_iter().enumerate() {
        let base = 128 + index * 40;
        let actual = ColumnGeometry {
            id: read_u32(header, base)?,
            kind: read_u32(header, base + 4)?,
            width: read_u32(header, base + 8)?,
            rows_per_block: read_u32(header, base + 12)?,
            first_descriptor: read_u32(header, base + 16)?,
            descriptor_count: read_u32(header, base + 20)?,
            offset: read_u64(header, base + 24)?,
            bytes: read_u64(header, base + 32)?,
        };
        if actual != expected {
            return Err(FormatError::Field);
        }
    }
    validate_descriptors(rows, Descriptors::Encoded(descriptor_bytes))?;
    Ok((
        UnitSummary {
            database,
            rows,
            projected_crc32c: read_u32(header, 112)?,
        },
        metadata_crc,
    ))
}

pub(super) fn decode_unit_metadata_into<'out>(
    header: &[u8],
    descriptor_bytes: &[u8],
    output: &'out mut std::mem::MaybeUninit<UnitMetadata>,
) -> Result<(&'out UnitMetadata, u32), FormatError> {
    let (summary, checksum) = decode_unit_summary(header, descriptor_bytes)?;
    let metadata = output.write(UnitMetadata {
        database: summary.database,
        rows: summary.rows,
        projected_crc32c: summary.projected_crc32c,
        descriptors: [BlockDescriptor::EMPTY; DESCRIPTOR_CAPACITY],
    });
    let count = usize::try_from(layout(summary.rows)?.descriptor_count)
        .map_err(|_| FormatError::Overflow)?;
    for (index, slot) in metadata.descriptors.iter_mut().enumerate().take(count) {
        *slot = Descriptors::Encoded(descriptor_bytes).get(index)?;
    }
    Ok((metadata, checksum))
}

#[cfg(test)]
pub(super) fn decode_unit_metadata(
    header: &[u8],
    descriptors: &[u8],
) -> Result<(UnitMetadata, u32), FormatError> {
    let mut output = std::mem::MaybeUninit::uninit();
    let (metadata, checksum) = decode_unit_metadata_into(header, descriptors, &mut output)?;
    Ok((*metadata, checksum))
}

#[cfg(test)]
pub(super) fn validate_unit_payload(
    metadata: &UnitMetadata,
    file: &[u8],
) -> Result<(), FormatError> {
    let geometry = layout(metadata.rows)?;
    if u64::try_from(file.len()).map_err(|_| FormatError::Overflow)? != geometry.unit_bytes {
        return Err(FormatError::Length);
    }
    require_zero(&file[HEADER_BYTES + DESCRIPTOR_BYTES..PAYLOAD_OFFSET as usize])?;
    for descriptor in &metadata.descriptors[..metadata.descriptor_count()] {
        let start = usize::try_from(descriptor.offset).map_err(|_| FormatError::Overflow)?;
        let length = usize::try_from(descriptor.bytes).map_err(|_| FormatError::Overflow)?;
        let end = start.checked_add(length).ok_or(FormatError::Overflow)?;
        let payload = file.get(start..end).ok_or(FormatError::Length)?;
        if crc32c(payload) != descriptor.crc32c {
            return Err(FormatError::Checksum);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ColumnGeometry {
    id: u32,
    kind: u32,
    width: u32,
    rows_per_block: u32,
    first_descriptor: u32,
    descriptor_count: u32,
    offset: u64,
    bytes: u64,
}

fn column_geometry(rows: u64) -> Result<[ColumnGeometry; 7], FormatError> {
    let geometry = layout(rows)?;
    // Format-4 column order: quantity, extended price, discount, tax,
    // return flag, line status, ship date. Each entry keeps its wire kind,
    // byte width, block row bound and derived block count together.
    let specifications = [
        (1_u32, 8_u64, DOUBLE_ROWS_PER_BLOCK, geometry.double_blocks),
        (1, 8, DOUBLE_ROWS_PER_BLOCK, geometry.double_blocks),
        (1, 8, DOUBLE_ROWS_PER_BLOCK, geometry.double_blocks),
        (1, 8, DOUBLE_ROWS_PER_BLOCK, geometry.double_blocks),
        (3, 1, DOUBLE_ROWS_PER_BLOCK, geometry.double_blocks),
        (3, 1, DOUBLE_ROWS_PER_BLOCK, geometry.double_blocks),
        (2, 4, DATE_ROWS_PER_BLOCK, geometry.date_blocks),
    ];
    let mut offset = PAYLOAD_OFFSET;
    let mut first_descriptor = 0_u32;
    let mut columns = [ColumnGeometry {
        id: 0,
        kind: 0,
        width: 0,
        rows_per_block: 0,
        first_descriptor: 0,
        descriptor_count: 0,
        offset: 0,
        bytes: 0,
    }; 7];
    for (index, (kind, width, rows_per_block, descriptor_count)) in
        specifications.into_iter().enumerate()
    {
        let bytes = rows.checked_mul(width).ok_or(FormatError::Overflow)?;
        columns[index] = ColumnGeometry {
            id: u32::try_from(index + 1).map_err(|_| FormatError::Overflow)?,
            kind,
            width: u32::try_from(width).map_err(|_| FormatError::Overflow)?,
            rows_per_block: u32::try_from(rows_per_block).map_err(|_| FormatError::Overflow)?,
            first_descriptor,
            descriptor_count,
            offset,
            bytes,
        };
        offset = offset.checked_add(bytes).ok_or(FormatError::Overflow)?;
        first_descriptor = first_descriptor
            .checked_add(descriptor_count)
            .ok_or(FormatError::Overflow)?;
    }
    if offset != geometry.unit_bytes || first_descriptor != geometry.descriptor_count {
        return Err(FormatError::Field);
    }
    Ok(columns)
}

enum Descriptors<'a> {
    Encoded(&'a [u8]),
    Decoded(&'a [BlockDescriptor; DESCRIPTOR_CAPACITY]),
}

impl Descriptors<'_> {
    fn get(&self, index: usize) -> Result<BlockDescriptor, FormatError> {
        if index >= DESCRIPTOR_CAPACITY {
            return Err(FormatError::Field);
        }
        match self {
            Self::Decoded(values) => Ok(values[index]),
            Self::Encoded(bytes) => {
                let offset = index.checked_mul(16).ok_or(FormatError::Overflow)?;
                Ok(BlockDescriptor {
                    offset: read_u64(bytes, offset)?,
                    bytes: read_u32(bytes, offset + 8)?,
                    crc32c: read_u32(bytes, offset + 12)?,
                })
            }
        }
    }
}

fn validate_descriptors(rows: u64, descriptors: Descriptors<'_>) -> Result<(), FormatError> {
    if let Descriptors::Encoded(bytes) = &descriptors {
        require_length(bytes, DESCRIPTOR_BYTES)?;
    }
    let columns = column_geometry(rows)?;
    let mut descriptor_index = 0_usize;
    for column in columns {
        let mut offset = column.offset;
        for block in 0..column.descriptor_count {
            let descriptor = descriptors.get(descriptor_index)?;
            let consumed_rows = u64::from(block)
                .checked_mul(u64::from(column.rows_per_block))
                .ok_or(FormatError::Overflow)?;
            let remaining_rows = rows
                .checked_sub(consumed_rows)
                .ok_or(FormatError::Overflow)?;
            let block_rows = remaining_rows.min(u64::from(column.rows_per_block));
            let expected_bytes = block_rows
                .checked_mul(u64::from(column.width))
                .ok_or(FormatError::Overflow)?;
            if descriptor.offset != offset
                || u64::from(descriptor.bytes) != expected_bytes
                || expected_bytes == 0
                || expected_bytes > BLOCK_BYTES
            {
                return Err(FormatError::Field);
            }
            offset = offset
                .checked_add(expected_bytes)
                .ok_or(FormatError::Overflow)?;
            descriptor_index = descriptor_index
                .checked_add(1)
                .ok_or(FormatError::Overflow)?;
        }
        if offset
            != column
                .offset
                .checked_add(column.bytes)
                .ok_or(FormatError::Overflow)?
        {
            return Err(FormatError::Field);
        }
    }
    if descriptor_index
        != usize::try_from(layout(rows)?.descriptor_count).map_err(|_| FormatError::Overflow)?
    {
        return Err(FormatError::Field);
    }
    for index in descriptor_index..DESCRIPTOR_CAPACITY {
        if descriptors.get(index)? != BlockDescriptor::EMPTY {
            return Err(FormatError::Field);
        }
    }
    Ok(())
}

pub(super) fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = Crc32c::new();
    crc.update(bytes);
    crc.finish()
}

#[derive(Clone, Copy)]
pub(super) struct Crc32c(u32);

impl Crc32c {
    pub(super) fn new() -> Self {
        Self(u32::MAX)
    }

    pub(super) fn update(&mut self, bytes: &[u8]) {
        let mut offset = 0_usize;
        while bytes.len() - offset >= 8 {
            let first = u32::from_le_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("CRC chunk has four bytes"),
            ) ^ self.0;
            let first_bytes = first.to_le_bytes();
            self.0 = CRC32C_TABLES[7][usize::from(first_bytes[0])]
                ^ CRC32C_TABLES[6][usize::from(first_bytes[1])]
                ^ CRC32C_TABLES[5][usize::from(first_bytes[2])]
                ^ CRC32C_TABLES[4][usize::from(first_bytes[3])]
                ^ CRC32C_TABLES[3][usize::from(bytes[offset + 4])]
                ^ CRC32C_TABLES[2][usize::from(bytes[offset + 5])]
                ^ CRC32C_TABLES[1][usize::from(bytes[offset + 6])]
                ^ CRC32C_TABLES[0][usize::from(bytes[offset + 7])];
            offset += 8;
        }
        for byte in &bytes[offset..] {
            let index = (self.0 as u8) ^ byte;
            self.0 = CRC32C_TABLES[0][usize::from(index)] ^ (self.0 >> 8);
        }
    }

    pub(super) fn finish(self) -> u32 {
        self.0 ^ u32::MAX
    }
}

fn metadata_crc32c(header: &[u8], descriptors: &[u8]) -> u32 {
    let mut crc = Crc32c::new();
    crc.update(&header[..100]);
    crc.update(&[0; 12]);
    crc.update(&header[112..]);
    crc.update(descriptors);
    crc.finish()
}

const CRC32C_TABLES: [[u32; 256]; 8] = make_crc32c_tables();

const fn make_crc32c_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0_u32; 256]; 8];
    let mut index = 0_usize;
    while index < tables[0].len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0x82f6_3b78
            } else {
                value >> 1
            };
            bit += 1;
        }
        tables[0][index] = value;
        index += 1;
    }
    let mut table = 1_usize;
    while table < tables.len() {
        index = 0;
        while index < tables[table].len() {
            let previous = tables[table - 1][index];
            tables[table][index] = (previous >> 8) ^ tables[0][(previous & 0xff) as usize];
            index += 1;
        }
        table += 1;
    }
    tables
}

fn require_version_length(bytes: &[u8], length: usize) -> Result<(), FormatError> {
    if read_u32(bytes, 8)? != FORMAT_VERSION {
        return Err(FormatError::Version);
    }
    if read_u32(bytes, 12)? != length as u32 {
        return Err(FormatError::Length);
    }
    Ok(())
}

fn require_length(bytes: &[u8], expected: usize) -> Result<(), FormatError> {
    if bytes.len() != expected {
        return Err(FormatError::Length);
    }
    Ok(())
}

fn require_zero(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes.iter().any(|byte| *byte != 0) {
        return Err(FormatError::Reserved);
    }
    Ok(())
}

fn require_checksum(bytes: &[u8], offset: usize) -> Result<(), FormatError> {
    let expected = read_u32(bytes, offset)?;
    let mut crc = Crc32c::new();
    crc.update(&bytes[..offset]);
    crc.update(&[0; 4]);
    crc.update(&bytes[offset + 4..]);
    if crc.finish() != expected {
        return Err(FormatError::Checksum);
    }
    Ok(())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, FormatError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(FormatError::Length)?
        .try_into()
        .map_err(|_| FormatError::Length)?;
    Ok(u32::from_le_bytes(value))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, FormatError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(FormatError::Length)?
        .try_into()
        .map_err(|_| FormatError::Length)?;
    Ok(u64::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_catalog_namespace_is_rejected_with_valid_checksums() {
        let mut control = *include_bytes!("../tests/fixtures/catalog-roots/CONTROL");
        put_u32(&mut control, 8, 6);
        put_u32(&mut control, 36, 0);
        let checksum = crc32c(&control);
        put_u32(&mut control, 36, checksum);
        assert_eq!(decode_control(&control), Err(FormatError::Version));
        for original in [
            include_bytes!("../tests/fixtures/catalog-roots/ROOT.A").as_slice(),
            include_bytes!("../tests/fixtures/catalog-roots/WAL").as_slice(),
        ] {
            let mut bytes = original.to_vec();
            put_u32(&mut bytes, 8, 6);
            put_u32(&mut bytes, 108, 0);
            let checksum = crc32c(&bytes);
            put_u32(&mut bytes, 108, checksum);
            if bytes.len() == ROOT_BYTES {
                assert_eq!(decode_root(&bytes), Err(FormatError::Version));
            } else {
                assert_eq!(decode_wal(&bytes), Err(FormatError::Version));
            }
        }
    }

    const GOLDEN_CONTROL: &[u8] =
        include_bytes!("../tests/fixtures/current-single-table-format/CONTROL");
    const GOLDEN_ROOT_A: &[u8] =
        include_bytes!("../tests/fixtures/current-single-table-format/ROOT.A");
    const GOLDEN_ROOT_B: &[u8] =
        include_bytes!("../tests/fixtures/current-single-table-format/ROOT.B");
    const GOLDEN_WAL: &[u8] = include_bytes!("../tests/fixtures/current-single-table-format/WAL");
    const GOLDEN_EMPTY_UNIT: &[u8] =
        include_bytes!("../tests/fixtures/current-single-table-format/EMPTY.UNIT");
    const GOLDEN_UNIT: &[u8] = include_bytes!("../tests/fixtures/current-single-table-format/UNIT");

    fn database() -> DatabaseId {
        DatabaseId::new(array::from_fn(|index| index as u8)).unwrap()
    }

    fn golden_unit_metadata() -> UnitMetadata {
        let payloads: [&[u8]; 7] = [
            &1.0_f64.to_le_bytes(),
            &2.0_f64.to_le_bytes(),
            &0.5_f64.to_le_bytes(),
            &0.25_f64.to_le_bytes(),
            b"A",
            b"F",
            &0_i32.to_le_bytes(),
        ];
        let mut offset = PAYLOAD_OFFSET;
        let descriptors = array::from_fn::<_, 7, _>(|index| {
            let payload = payloads[index];
            let descriptor = BlockDescriptor {
                offset,
                bytes: payload.len() as u32,
                crc32c: crc32c(payload),
            };
            offset += payload.len() as u64;
            descriptor
        });
        let projected: Vec<u8> = payloads.into_iter().flatten().copied().collect();
        UnitMetadata::new(database(), 1, crc32c(&projected), &descriptors).unwrap()
    }

    #[test]
    fn crc32c_matches_independent_check_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn slicing_crc_matches_bitwise_oracle_at_offsets_and_partitions() {
        fn bitwise(bytes: &[u8]) -> u32 {
            let mut crc = u32::MAX;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0x82f6_3b78
                    } else {
                        crc >> 1
                    };
                }
            }
            crc ^ u32::MAX
        }

        let bytes: Vec<u8> = (0_u16..=1_030)
            .map(|value| value.wrapping_mul(157).to_le_bytes()[0])
            .collect();
        for length in 0..=bytes.len() {
            assert_eq!(crc32c(&bytes[..length]), bitwise(&bytes[..length]));
            let mut partitioned = Crc32c::new();
            for chunk in bytes[..length].chunks(7) {
                partitioned.update(chunk);
            }
            assert_eq!(partitioned.finish(), bitwise(&bytes[..length]));
        }
    }

    #[test]
    fn checked_layout_matches_all_independent_vectors() {
        let vectors = [
            (0, 0, 0, 28_672, 28_672, 0),
            (1, 7, 38, 28_710, 28_748, 1),
            (32_768, 7, 1_245_184, 1_273_856, 2_519_040, 1),
            (65_536, 13, 2_490_368, 2_519_040, 5_009_408, 2),
            (6_001_215, 1_196, 228_046_170, 228_074_842, 456_121_012, 184),
            (6_500_000, 1_294, 247_000_000, 247_028_672, 494_028_672, 199),
        ];
        for (rows, descriptors, payload, unit, temporary, double_blocks) in vectors {
            let actual = layout(rows).unwrap();
            assert_eq!(actual.descriptor_count, descriptors);
            assert_eq!(actual.payload_bytes, payload);
            assert_eq!(actual.unit_bytes, unit);
            assert_eq!(actual.temporary_peak_bytes, temporary);
            assert_eq!(actual.double_blocks, double_blocks);
        }
        assert_eq!(layout(MAX_ROWS + 1), Err(FormatError::Field));
        assert_eq!(layout(u64::MAX), Err(FormatError::Field));
    }

    fn descriptors_for(rows: u64) -> Vec<BlockDescriptor> {
        let columns = column_geometry(rows).unwrap();
        let mut descriptors = Vec::new();
        for column in columns {
            let mut offset = column.offset;
            for block in 0..column.descriptor_count {
                let consumed = u64::from(block)
                    .checked_mul(u64::from(column.rows_per_block))
                    .unwrap();
                let block_rows = rows
                    .checked_sub(consumed)
                    .unwrap()
                    .min(u64::from(column.rows_per_block));
                let bytes = block_rows.checked_mul(u64::from(column.width)).unwrap();
                descriptors.push(BlockDescriptor {
                    offset,
                    bytes: u32::try_from(bytes).unwrap(),
                    crc32c: 0,
                });
                offset = offset.checked_add(bytes).unwrap();
            }
        }
        descriptors
    }

    #[test]
    fn block_descriptors_validate_boundary_geometry() {
        for rows in [0_u64, 1, 32_768, 32_769, 65_536, 65_537, MAX_ROWS] {
            let descriptors = descriptors_for(rows);
            let metadata = UnitMetadata::new(database(), rows, 0, &descriptors).unwrap();
            assert_eq!(metadata.descriptor_count(), descriptors.len());
        }
        let maximum_descriptors = descriptors_for(MAX_ROWS);
        let maximum = UnitMetadata::new(database(), MAX_ROWS, 0, &maximum_descriptors).unwrap();
        let (header, encoded, checksum) = encode_unit_metadata(&maximum).unwrap();
        let (decoded, decoded_checksum) = decode_unit_metadata(&header, &encoded).unwrap();
        assert_eq!(decoded, maximum);
        assert_eq!(decoded_checksum, checksum);

        let mut descriptors = descriptors_for(32_769);
        descriptors[1].bytes = descriptors[1].bytes.checked_sub(1).unwrap();
        assert_eq!(
            UnitMetadata::new(database(), 32_769, 0, &descriptors),
            Err(FormatError::Field)
        );
    }

    #[test]
    fn codecs_reproduce_independent_golden_bytes() {
        let database = database();
        assert_eq!(encode_control(database), GOLDEN_CONTROL);
        assert_eq!(decode_control(GOLDEN_CONTROL), Ok(database));
        let metadata = golden_unit_metadata();
        let (header, descriptors, metadata_crc) = encode_unit_metadata(&metadata).unwrap();
        assert_eq!(&header, &GOLDEN_UNIT[..HEADER_BYTES]);
        assert_eq!(
            &descriptors,
            &GOLDEN_UNIT[HEADER_BYTES..HEADER_BYTES + DESCRIPTOR_BYTES]
        );
        let (decoded, decoded_crc) = decode_unit_metadata(&header, &descriptors).unwrap();
        assert_eq!(decoded, metadata);
        assert_eq!(decoded_crc, metadata_crc);
        validate_unit_payload(&decoded, GOLDEN_UNIT).unwrap();
        let empty = UnitMetadata::new(database, 0, 0, &[]).unwrap();
        let (empty_header, empty_descriptors, empty_crc) = encode_unit_metadata(&empty).unwrap();
        assert_eq!(&empty_header, &GOLDEN_EMPTY_UNIT[..HEADER_BYTES]);
        assert_eq!(
            &empty_descriptors,
            &GOLDEN_EMPTY_UNIT[HEADER_BYTES..HEADER_BYTES + DESCRIPTOR_BYTES]
        );
        let (decoded_empty, decoded_empty_crc) =
            decode_unit_metadata(&empty_header, &empty_descriptors).unwrap();
        assert_eq!(decoded_empty, empty);
        assert_eq!(decoded_empty_crc, empty_crc);
        validate_unit_payload(&decoded_empty, GOLDEN_EMPTY_UNIT).unwrap();

        let record = WalRecord {
            database,
            issued: 2,
            state: RootState::Data {
                transaction: TransactionId::for_attempt(database, 2).unwrap(),
                rows: 1,
                unit_bytes: GOLDEN_UNIT.len() as u64,
                unit_metadata_crc32c: metadata_crc,
            },
        };
        assert_eq!(encode_wal(record).unwrap(), GOLDEN_WAL);
        assert_eq!(decode_wal(GOLDEN_WAL), Ok(record));
        for (replica, golden) in [(Replica::A, GOLDEN_ROOT_A), (Replica::B, GOLDEN_ROOT_B)] {
            let root = Root {
                database,
                issued: record.issued,
                replica,
                state: record.state,
            };
            assert_eq!(encode_root(root).unwrap(), golden);
            assert_eq!(decode_root(golden), Ok(root));
        }
    }

    #[test]
    fn rejected_attempt_reuse_bytes_fail_closed() {
        assert_eq!(
            decode_control(include_bytes!(
                "../tests/fixtures/rejected-attempt-reuse-format/CONTROL"
            )),
            Err(FormatError::Version)
        );
        for bytes in [
            include_bytes!("../tests/fixtures/rejected-attempt-reuse-format/ROOT.A"),
            include_bytes!("../tests/fixtures/rejected-attempt-reuse-format/ROOT.B"),
        ] {
            assert_eq!(decode_root(bytes), Err(FormatError::Version));
            assert_eq!(decode_root(&bytes[..12]), Err(FormatError::Version));
        }
        assert_eq!(
            decode_wal(include_bytes!(
                "../tests/fixtures/rejected-attempt-reuse-format/WAL"
            )),
            Err(FormatError::Version)
        );
    }

    #[test]
    fn snapshot_selection_requires_complete_authority() {
        let database = database();
        let empty = |issued| WalRecord {
            database,
            issued,
            state: RootState::Empty,
        };
        let committed = |issued| WalRecord {
            database,
            issued,
            state: RootState::Data {
                transaction: TransactionId::for_attempt(database, issued).unwrap(),
                rows: 0,
                unit_bytes: PAYLOAD_OFFSET,
                unit_metadata_crc32c: 7,
            },
        };
        let replica = |snapshot: WalRecord, replica| Root {
            database: snapshot.database,
            issued: snapshot.issued,
            state: snapshot.state,
            replica,
        };
        let snapshots = [empty(0), empty(1), committed(1), empty(2), committed(2)];
        let mut cases = 0;
        for a in 0..=snapshots.len() {
            for b in 0..=snapshots.len() {
                for w in 0..=snapshots.len() {
                    let left = snapshots.get(a).copied();
                    let right = snapshots.get(b).copied();
                    let fence = snapshots.get(w).copied();
                    // Independent finite transition relation, including forks.
                    let adjacent =
                        |old, next| matches!((old, next), (0, 1) | (1, 2) | (1, 3) | (3, 4));
                    let selected = if a == b && left.is_some() {
                        Some(a)
                    } else if adjacent(a, b) {
                        Some(b)
                    } else if adjacent(b, a) || (left.is_some() && right.is_none() && a == w) {
                        Some(a)
                    } else if right.is_some() && left.is_none() && b == w {
                        Some(b)
                    } else {
                        None
                    };
                    let expected = selected.filter(|selected| {
                        fence.is_none() || *selected == w || adjacent(*selected, w)
                    });
                    let actual = select_snapshot(
                        left.map(|value| replica(value, Replica::A)),
                        right.map(|value| replica(value, Replica::B)),
                        fence,
                    );
                    assert_eq!(
                        actual.as_ref().ok().map(|(value, _)| *value),
                        expected.map(|index| snapshots[index]),
                        "a={a} b={b} w={w}"
                    );
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 216);
    }

    #[test]
    fn receipt_identity_and_issued_prefix_are_independently_checked() {
        let database = database();
        for (issued, sequence, valid) in [
            (0, 1, false),
            (1, 2, false),
            (1, 1, true),
            (2, 1, true),
            (u64::MAX, u64::MAX, true),
        ] {
            let snapshot = WalRecord {
                database,
                issued,
                state: RootState::Data {
                    transaction: TransactionId::for_attempt(database, sequence).unwrap(),
                    rows: 0,
                    unit_bytes: PAYLOAD_OFFSET,
                    unit_metadata_crc32c: 9,
                },
            };
            assert_eq!(encode_wal(snapshot).is_ok(), valid);
        }
        let snapshot = WalRecord {
            database,
            issued: 2,
            state: RootState::Data {
                transaction: TransactionId::for_attempt(database, 2).unwrap(),
                rows: 0,
                unit_bytes: PAYLOAD_OFFSET,
                unit_metadata_crc32c: 9,
            },
        };
        for (offset, value) in [(112, 1_u64), (72, 0), (56, u64::MAX)] {
            let mut bytes = encode_wal(snapshot).unwrap();
            put_u64(&mut bytes, offset, value);
            put_u32(&mut bytes, 108, 0);
            let checksum = crc32c(&bytes);
            put_u32(&mut bytes, 108, checksum);
            assert!(decode_wal(&bytes).is_err());
        }
    }

    #[test]
    fn empty_roots_are_canonical_and_replica_specific() {
        let database = database();
        let a = Root {
            database,
            issued: 0,
            replica: Replica::A,
            state: RootState::Empty,
        };
        let b = Root {
            database,
            issued: 0,
            replica: Replica::B,
            state: RootState::Empty,
        };
        let a_bytes = encode_root(a).unwrap();
        let b_bytes = encode_root(b).unwrap();
        assert_ne!(a_bytes, b_bytes);
        assert_eq!(decode_root(&a_bytes), Ok(a));
        assert_eq!(decode_root(&b_bytes), Ok(b));
    }

    #[test]
    fn newer_generations_are_distinct_from_corruption() {
        let mut root = encode_root(Root {
            database: database(),
            issued: 0,
            replica: Replica::A,
            state: RootState::Empty,
        })
        .unwrap();
        put_u64(&mut root, 40, 2);
        put_u32(&mut root, 108, 0);
        let root_crc = crc32c(&root);
        put_u32(&mut root, 108, root_crc);
        assert_eq!(decode_root(&root), Err(FormatError::Generation(2)));

        let mut wal = GOLDEN_WAL.to_owned();
        put_u64(&mut wal, 40, 2);
        put_u32(&mut wal, 108, 0);
        let wal_crc = crc32c(&wal);
        put_u32(&mut wal, 108, wal_crc);
        assert_eq!(decode_wal(&wal), Err(FormatError::Generation(2)));

        let metadata = golden_unit_metadata();
        let (mut header, descriptors, _) = encode_unit_metadata(&metadata).unwrap();
        put_u64(&mut header, 32, 2);
        assert_eq!(
            decode_unit_metadata(&header, &descriptors),
            Err(FormatError::Generation(2))
        );
    }

    #[test]
    fn decoder_slot_reuse_after_partial_validation_failure() {
        let expected = golden_unit_metadata();
        let (mut header, mut descriptors, checksum) = encode_unit_metadata(&expected).unwrap();
        let mut bad_header = header;
        let mut bad_descriptors = descriptors;
        put_u64(&mut bad_descriptors, 0, PAYLOAD_OFFSET + 1);
        put_u32(&mut bad_header, 100, 0);
        let descriptor_crc = crc32c(&bad_descriptors);
        put_u32(&mut bad_header, 104, descriptor_crc);
        let metadata_crc = metadata_crc32c(&bad_header, &bad_descriptors);
        put_u32(&mut bad_header, 108, metadata_crc);
        let header_crc = crc32c(&bad_header);
        put_u32(&mut bad_header, 100, header_crc);
        let mut output = std::mem::MaybeUninit::uninit();
        // Checksums and header shape pass; descriptor validation fails after
        // initialization. No trusted reference escapes that failed constructor.
        assert!(matches!(
            decode_unit_metadata_into(&bad_header, &bad_descriptors, &mut output),
            Err(FormatError::Field)
        ));
        let (decoded, actual_checksum) =
            decode_unit_metadata_into(&header, &descriptors, &mut output).unwrap();
        header.fill(0);
        descriptors.fill(0);
        assert_eq!(decoded, &expected);
        assert_eq!(actual_checksum, checksum);
    }

    #[test]
    fn prefixes_trailing_and_single_byte_mutations_fail() {
        let files = [GOLDEN_CONTROL, GOLDEN_ROOT_A, GOLDEN_ROOT_B, GOLDEN_WAL];
        for bytes in files {
            for length in 0..bytes.len() {
                assert!(decode_by_length(&bytes[..length]).is_err());
            }
            let mut trailing = bytes.to_vec();
            trailing.push(0);
            assert!(decode_by_length(&trailing).is_err());
            for index in 0..bytes.len() {
                let mut changed = bytes.to_vec();
                changed[index] ^= 0x80;
                assert!(decode_by_length(&changed).is_err(), "mutation {index}");
            }
        }
        for length in 0..GOLDEN_UNIT.len() {
            assert!(decode_complete_unit(&GOLDEN_UNIT[..length]).is_err());
        }
        let mut trailing = GOLDEN_UNIT.to_vec();
        trailing.push(0);
        assert!(decode_complete_unit(&trailing).is_err());
        for index in 0..GOLDEN_UNIT.len() {
            let mut changed = GOLDEN_UNIT.to_vec();
            changed[index] ^= 0x80;
            assert!(
                decode_complete_unit(&changed).is_err(),
                "unit mutation {index}"
            );
        }
    }

    fn decode_by_length(bytes: &[u8]) -> Result<(), FormatError> {
        match bytes.len() {
            CONTROL_BYTES => decode_control(bytes).map(|_| ()),
            WAL_BYTES => decode_wal(bytes).map(|_| ()),
            ROOT_BYTES => decode_root(bytes).map(|_| ()),
            _ => Err(FormatError::Length),
        }
    }

    fn decode_complete_unit(bytes: &[u8]) -> Result<(), FormatError> {
        if bytes.len() < HEADER_BYTES + DESCRIPTOR_BYTES {
            return Err(FormatError::Length);
        }
        let (metadata, _) = decode_unit_metadata(
            &bytes[..HEADER_BYTES],
            &bytes[HEADER_BYTES..HEADER_BYTES + DESCRIPTOR_BYTES],
        )?;
        validate_unit_payload(&metadata, bytes)
    }

    #[test]
    fn checksum_consistent_semantic_corruption_fails() {
        let metadata = golden_unit_metadata();
        let mut overlap = metadata;
        overlap.descriptors[1].offset = overlap.descriptors[0].offset;
        assert_eq!(
            validate_descriptors(overlap.rows, Descriptors::Decoded(&overlap.descriptors)),
            Err(FormatError::Field)
        );
        let mut unused = metadata;
        unused.descriptors[7].bytes = 1;
        assert_eq!(
            validate_descriptors(unused.rows, Descriptors::Decoded(&unused.descriptors)),
            Err(FormatError::Field)
        );
        assert_eq!(DatabaseId::new([0; 16]), Err(FormatError::Identity));
    }
}
