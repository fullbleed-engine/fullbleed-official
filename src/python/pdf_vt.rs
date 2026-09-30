use super::*;
use crate::python_abi::IntoPyValue;

fn value_to_py(py: Python<'_>, value: &crate::DpmValue) -> PyResult<PyObject> {
    use crate::DpmValue;
    match value {
        DpmValue::String(value) => value.clone().into_py_value(),
        DpmValue::Boolean(value) => value.into_py_value(),
        DpmValue::Integer(value) => value.into_py_value(),
        DpmValue::Real(value) => value.into_py_value(),
        DpmValue::Array(values) => {
            let items = PyList::empty(py);
            for value in values {
                items.append(value_to_py(py, value)?)?;
            }
            Ok(items.unbind().into_any())
        }
        DpmValue::Dictionary(values) => {
            let result = PyDict::new(py);
            for (key, value) in values {
                result.set_item(key, value_to_py(py, value)?)?;
            }
            Ok(result.unbind().into_any())
        }
    }
}

pub(super) fn parts_to_py(
    py: Python<'_>,
    parts: &[crate::pdfinspect::PdfVtPartInspection],
) -> PyResult<PyObject> {
    let result = PyList::empty(py);
    for part in parts {
        let item = PyDict::new(py);
        item.set_item("object_id", part.object_id)?;
        item.set_item("depth", part.depth)?;
        item.set_item("name", part.name.clone())?;
        item.set_item("id", part.id.clone())?;
        item.set_item("first_page", part.first_page)?;
        item.set_item("last_page", part.last_page)?;
        item.set_item(
            "metadata",
            value_to_py(py, &crate::DpmValue::Dictionary(part.metadata.clone()))?,
        )?;
        result.append(item)?;
    }
    Ok(result.unbind().into_any())
}

fn parse_metadata(value: Option<&Bound<'_, PyAny>>) -> PyResult<crate::DpmMetadata> {
    fn parse(
        value: &Bound<'_, PyAny>,
        depth: usize,
        count: &mut usize,
    ) -> PyResult<crate::DpmValue> {
        use crate::DpmValue;
        *count += 1;
        if depth > 16 || *count > 10000 {
            return Err(PyValueError::new_err(
                "PDF_VT_JOB_INVALID: DPM exceeds depth 16 or 10000 values per node",
            ));
        }
        if let Ok(value) = value.extract::<String>() {
            return Ok(DpmValue::String(value));
        }
        if let Ok(value) = value.extract::<bool>() {
            return Ok(DpmValue::Boolean(value));
        }
        if let Ok(dict) = value.downcast::<PyDict>() {
            let mut result = BTreeMap::new();
            for (key, value) in dict.iter() {
                result.insert(key.extract::<String>()?, parse(&value, depth + 1, count)?);
            }
            return Ok(DpmValue::Dictionary(result));
        }
        if let Ok(items) = value.downcast::<PyList>() {
            return Ok(DpmValue::Array(
                items
                    .iter()
                    .map(|item| parse(&item, depth + 1, count))
                    .collect::<PyResult<_>>()?,
            ));
        }
        if value.is_builtin_instance("int")? {
            return value.extract::<i64>().map(DpmValue::Integer).map_err(|_| {
                PyValueError::new_err(
                    "PDF_VT_JOB_INVALID: DPM integers must fit signed 64-bit values",
                )
            });
        }
        if value.is_builtin_instance("float")? {
            return value.extract::<f64>().map(DpmValue::Real);
        }
        Err(PyValueError::new_err(
            "PDF_VT_JOB_INVALID: DPM values must be strings, booleans, numbers, lists or dictionaries",
        ))
    }
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    match parse(value, 0, &mut 0)? {
        crate::DpmValue::Dictionary(value) => Ok(value),
        _ => Err(PyValueError::new_err(
            "PDF_VT_JOB_INVALID: metadata must be a dictionary",
        )),
    }
}

pub(super) fn parse_job(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<crate::PdfVtJob>> {
    fn fields(
        value: &Bound<'_, PyAny>,
        allowed: &[&str],
    ) -> PyResult<(String, crate::DpmMetadata)> {
        let dict = value.downcast::<PyDict>()?;
        for (key, _) in dict.iter() {
            let key = key.extract::<String>()?;
            if !allowed.contains(&key.as_str()) {
                return Err(PyValueError::new_err(format!(
                    "PDF_VT_JOB_INVALID: unknown field {key:?}"
                )));
            }
        }
        let id = dict
            .get_item("id")?
            .ok_or_else(|| PyValueError::new_err("PDF_VT_JOB_INVALID: id is required"))?
            .extract::<String>()?;
        let metadata = parse_metadata(dict.get_item("metadata")?.as_ref())?;
        Ok((id, metadata))
    }
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    let (id, metadata) = fields(value, &["id", "metadata", "records"])?;
    let mut job = crate::PdfVtJob {
        id,
        metadata,
        records: Vec::new(),
    };
    if let Some(records) = value.downcast::<PyDict>()?.get_item("records")? {
        for record in records.downcast::<PyList>()?.iter() {
            let (id, metadata) = fields(&record, &["id", "metadata", "documents"])?;
            let mut result = crate::PdfVtRecord {
                id,
                metadata,
                documents: Vec::new(),
            };
            let documents = record
                .downcast::<PyDict>()?
                .get_item("documents")?
                .ok_or_else(|| {
                    PyValueError::new_err("PDF_VT_JOB_INVALID: record documents are required")
                })?;
            for document in documents.downcast::<PyList>()?.iter() {
                let (id, metadata) = fields(&document, &["id", "metadata"])?;
                result.documents.push(crate::PdfVtDocument { id, metadata });
            }
            job.records.push(result);
        }
    }
    job.validate()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(Some(job))
}
