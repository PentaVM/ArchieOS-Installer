//! Provenance-bearing metrics and versioned vector records. No model output is
//! promoted to authority by writing one of these records.
use crate::{Result, StateError, StateStore};
use archie_memory::{cognitive::reference, graph::Sensitivity};
#[derive(Debug, Clone, PartialEq)]
pub struct VectorMatch {
    pub object_id: String,
    pub distance: f64,
    pub provenance_ref: String,
    pub recorded_at_ms: i64,
}
fn vector(values: &[f32]) -> Result<String> {
    if values.is_empty() || values.len() > 4096 || values.iter().any(|v| !v.is_finite()) {
        return Err(StateError::Corrupt(
            "invalid embedding dimension/value".into(),
        ));
    }
    Ok(format!(
        "[{}]",
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ))
}
impl StateStore {
    pub fn record_metric(
        &self,
        node: &str,
        metric: &str,
        time: i64,
        value: f64,
        provenance: &str,
    ) -> Result<()> {
        if !reference(node)
            || !reference(metric)
            || !reference(provenance)
            || time <= 0
            || !value.is_finite()
        {
            return Err(StateError::Corrupt("invalid metric".into()));
        }
        self.with_connection(|tx| {
            let inserted=tx.execute("INSERT INTO metric_samples VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",&[&time,&metric,&node,&value,&provenance])?;
            if inserted==0 {
                let row=tx.query_one("SELECT value,provenance_ref FROM metric_samples WHERE recorded_at=$1 AND metric=$2 AND node_id=$3",&[&time,&metric,&node])?;
                if row.try_get::<_,f64>(0)?!=value || row.try_get::<_,String>(1)?!=provenance { return Err(StateError::Corrupt("metric identity conflict".into())); }
            }
            Ok(())
        })
    }
    pub fn metric_mean(
        &self,
        node: &str,
        metric: &str,
        from: i64,
        through: i64,
    ) -> Result<Option<f64>> {
        if !reference(node) || !reference(metric) || from <= 0 || through < from {
            return Err(StateError::Corrupt("invalid metric interval".into()));
        }
        self.with_connection(|tx| Ok(tx.query_one("SELECT avg(value) FROM metric_samples WHERE node_id=$1 AND metric=$2 AND recorded_at BETWEEN $3 AND $4",&[&node,&metric,&from,&through])?.try_get(0)?))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn record_embedding(
        &self,
        object: &str,
        model: &str,
        values: &[f32],
        sensitivity: Sensitivity,
        provenance: &str,
        time: i64,
    ) -> Result<()> {
        if !reference(object) || !reference(model) || !reference(provenance) || time <= 0 {
            return Err(StateError::Corrupt("invalid embedding references".into()));
        }
        let value = vector(values)?;
        let dimensions = values.len() as i32;
        let sensitivity = sensitivity as i32;
        self.with_connection(|tx| {
            let inserted=tx.execute("INSERT INTO memory_vectors(object_id,model_id,dimensions,embedding,sensitivity,provenance_ref,recorded_at) VALUES($1,$2,$3,$4::text::public.vector,$5,$6,$7) ON CONFLICT DO NOTHING",&[&object,&model,&dimensions,&value,&sensitivity,&provenance,&time])?;
            if inserted==0 {
                let same:bool=tx.query_one("SELECT embedding OPERATOR(public.=) $4::text::public.vector AND sensitivity=$5 AND provenance_ref=$6 FROM memory_vectors WHERE object_id=$1 AND model_id=$2 AND recorded_at=$3",&[&object,&model,&time,&value,&sensitivity,&provenance])?.try_get(0)?;
                if !same { return Err(StateError::Corrupt("embedding identity conflict".into())); }
            }
            Ok(())
        })
    }
    pub fn nearest_embeddings(
        &self,
        model: &str,
        values: &[f32],
        as_of: i64,
        allowed: &[Sensitivity],
        limit: u32,
    ) -> Result<Vec<VectorMatch>> {
        if !reference(model) || as_of <= 0 || limit == 0 || limit > 100 || allowed.len() > 6 {
            return Err(StateError::Corrupt("invalid vector query".into()));
        }
        if allowed.is_empty() {
            return Ok(vec![]);
        }
        let value = vector(values)?;
        let dimensions = values.len() as i32;
        let labels: Vec<i32> = allowed.iter().map(|s| *s as i32).collect();
        self.with_connection(|tx| tx.query("WITH latest AS (SELECT DISTINCT ON(object_id) object_id,embedding,sensitivity,provenance_ref,recorded_at,dimensions FROM memory_vectors WHERE model_id=$1 AND recorded_at<=$2 ORDER BY object_id,recorded_at DESC) SELECT object_id,embedding OPERATOR(public.<->) $3::text::public.vector,provenance_ref,recorded_at FROM latest WHERE dimensions=$4 AND sensitivity=ANY($5) ORDER BY 2,1 LIMIT $6",&[&model,&as_of,&value,&dimensions,&labels,&i64::from(limit)])?.iter().map(|r| Ok(VectorMatch { object_id:r.try_get(0)?,distance:r.try_get(1)?,provenance_ref:r.try_get(2)?,recorded_at_ms:r.try_get(3)? })).collect())
    }
}
