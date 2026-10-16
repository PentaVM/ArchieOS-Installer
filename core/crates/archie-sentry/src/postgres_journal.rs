//! Uses Core's PostgreSQL connection and writer fence, with a separate migration.
use crate::{
    contracts::{validate_id, Intent},
    engine::ExecutionResult,
    journal::{Journal, JournalError, JournalRecord},
};
use archie_postgres::PostgresStore;
use archie_types::Timestamp;
use postgres::{GenericClient, Row};
use sha2::{Digest, Sha256};

const SCHEMA: &str = include_str!("sentry_schema.sql");
const LIMIT: usize = 1_048_576;
const COLUMNS: &str = "intent_id,operation_id,resource_id,intent_json,intent_sha256,reserved_at,result_json,result_sha256";
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

pub struct PostgresJournal<'a> {
    store: &'a PostgresStore,
}
impl<'a> PostgresJournal<'a> {
    pub fn open(store: &'a PostgresStore) -> Result<Self, JournalError> {
        store.with_transaction(|tx| -> Result<(), JournalError> {
            tx.batch_execute("CREATE TABLE IF NOT EXISTS sentry_schema_migrations (version INTEGER PRIMARY KEY, schema_sha256 TEXT NOT NULL); REVOKE ALL ON sentry_schema_migrations FROM PUBLIC;")?;
            let versions=tx.query("SELECT version,schema_sha256 FROM sentry_schema_migrations ORDER BY version LIMIT 2", &[])?;
            let digest=hash(&SCHEMA.replace("\r\n", "\n"));
            if versions.is_empty() {
                tx.batch_execute(SCHEMA)?;
                tx.execute("INSERT INTO sentry_schema_migrations VALUES (1,$1)", &[&digest])?;
            } else if versions.len()!=1 || versions[0].try_get::<_,i32>(0)?!=1 || versions[0].try_get::<_,String>(1)?!=digest {
                return Err(JournalError::Corrupt);
            }
            Ok(())
        })?;
        Ok(Self { store })
    }
}
fn decode(row: Row) -> Result<(Intent, JournalRecord), JournalError> {
    let intent_id: String = row.try_get(0)?;
    let operation: i64 = row.try_get(1)?;
    let resource: String = row.try_get(2)?;
    let json: String = row.try_get(3)?;
    let digest: String = row.try_get(4)?;
    let reserved_at: i64 = row.try_get(5)?;
    if json.len() > LIMIT || hash(&json) != digest || operation <= 0 || reserved_at <= 0 {
        return Err(JournalError::Corrupt);
    }
    let intent: Intent = serde_json::from_str(&json).map_err(|_| JournalError::Corrupt)?;
    if intent.digest().map_err(|_| JournalError::Corrupt)? != digest
        || intent.intent_id != intent_id
        || intent.operation_id != operation as u64
        || intent.resource_id != resource
    {
        return Err(JournalError::Corrupt);
    }
    let payload: Option<String> = row.try_get(6)?;
    let checksum: Option<String> = row.try_get(7)?;
    let result = match (payload, checksum) {
        (None, None) => None,
        (Some(json), Some(checksum)) if json.len() <= LIMIT && hash(&json) == checksum => {
            let receipt: ExecutionResult =
                serde_json::from_str(&json).map_err(|_| JournalError::Corrupt)?;
            receipt.validate_for(&intent)?;
            if receipt.replayed {
                return Err(JournalError::Corrupt);
            }
            Some(receipt)
        }
        _ => return Err(JournalError::Corrupt),
    };
    Ok((
        intent,
        JournalRecord {
            intent_id,
            operation_id: operation as u64,
            digest,
            reserved_at: Timestamp(reserved_at),
            result,
        },
    ))
}
fn find(
    client: &mut impl GenericClient,
    id: &str,
) -> Result<Option<(Intent, JournalRecord)>, JournalError> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM sentry_intents WHERE intent_id=$1"),
            &[&id],
        )?
        .map(decode)
        .transpose()
}
impl Journal for PostgresJournal<'_> {
    fn load(&mut self, id: &str) -> Result<Option<JournalRecord>, JournalError> {
        validate_id(id).map_err(|_| JournalError::Corrupt)?;
        self.store
            .with_client(|client| Ok(find(client, id)?.map(|(_, record)| record)))
    }
    fn reserve(
        &mut self,
        intent: &Intent,
        digest: &str,
        at: Timestamp,
    ) -> Result<Option<JournalRecord>, JournalError> {
        if intent.digest().map_err(|_| JournalError::Corrupt)? != digest || at.0 <= 0 {
            return Err(JournalError::Corrupt);
        }
        let json = serde_json::to_string(intent).map_err(|_| JournalError::Corrupt)?;
        if json.len() > LIMIT {
            return Err(JournalError::Full);
        }
        self.store.with_transaction(|tx| {
            let count=tx.execute("INSERT INTO sentry_intents(intent_id,operation_id,resource_id,intent_json,intent_sha256,reserved_at) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
                &[&intent.intent_id,&(intent.operation_id as i64),&intent.resource_id,&json,&digest,&at.0])?;
            if count==1 {return Ok(None);}
            let record=find(tx,&intent.intent_id)?.ok_or(JournalError::Collision)?.1;
            if record.operation_id!=intent.operation_id || record.digest!=digest {return Err(JournalError::Collision);}
            Ok(Some(record))
        })
    }
    fn finish(&mut self, result: &ExecutionResult) -> Result<(), JournalError> {
        if result.replayed {
            return Err(JournalError::Corrupt);
        }
        let json = serde_json::to_string(result).map_err(|_| JournalError::Corrupt)?;
        if json.len() > LIMIT {
            return Err(JournalError::Full);
        }
        self.store.with_transaction(|tx| {
            let (intent,record)=find(tx,&result.intent_id)?.ok_or(JournalError::Corrupt)?;
            result.validate_for(&intent)?;
            if let Some(previous)=record.result { return if previous==*result {Ok(())} else {Err(JournalError::Collision)}; }
            if tx.execute("UPDATE sentry_intents SET result_json=$1,result_sha256=$2 WHERE intent_id=$3 AND operation_id=$4 AND intent_sha256=$5 AND result_json IS NULL", &[&json,&hash(&json),&result.intent_id,&(result.operation_id as i64),&result.intent_digest])?!=1 {return Err(JournalError::Collision);}
            Ok(())
        })
    }
}
