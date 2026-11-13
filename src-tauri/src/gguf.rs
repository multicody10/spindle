use crate::domain::ModelMetadata;
use serde_json::Value;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_STRING_BYTES: usize = 4 * 1024 * 1024;
const MAX_METADATA_ENTRIES: u64 = 1_000_000;
const MAX_ARRAY_ENTRIES: u64 = 10_000_000;

fn read_u32<R: Read>(reader: &mut R) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64<R: Read>(reader: &mut R) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_string<R: Read>(reader: &mut R, max_bytes: usize) -> io::Result<String> {
    let length = usize::try_from(read_u64(reader)?).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "GGUF string length is too large",
        )
    })?;
    if length > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "GGUF metadata string exceeds safety limit",
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "GGUF metadata string is not UTF-8",
        )
    })
}

fn skip_bytes<R: Seek>(reader: &mut R, count: u64) -> io::Result<()> {
    let current = reader.stream_position()?;
    let file_length = reader.seek(SeekFrom::End(0))?;
    let target = current
        .checked_add(count)
        .filter(|target| *target <= file_length)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "GGUF metadata value extends beyond the file",
            )
        })?;
    reader.seek(SeekFrom::Start(target))?;
    Ok(())
}

fn fixed_value_size(kind: u32) -> Option<u64> {
    match kind {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

fn skip_value<R: Read + Seek>(reader: &mut R, kind: u32, depth: usize) -> io::Result<()> {
    if depth > 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "GGUF metadata nesting exceeds safety limit",
        ));
    }
    if let Some(bytes) = fixed_value_size(kind) {
        return skip_bytes(reader, bytes);
    }
    match kind {
        8 => {
            let length = read_u64(reader)?;
            if length > MAX_STRING_BYTES as u64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GGUF string exceeds safety limit",
                ));
            }
            skip_bytes(reader, length)
        }
        9 => {
            let element_type = read_u32(reader)?;
            let count = read_u64(reader)?;
            if count > MAX_ARRAY_ENTRIES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GGUF metadata array exceeds safety limit",
                ));
            }
            if let Some(bytes) = fixed_value_size(element_type) {
                return skip_bytes(reader, bytes.saturating_mul(count));
            }
            for _ in 0..count {
                skip_value(reader, element_type, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unsupported GGUF metadata value type {kind}"),
        )),
    }
}

fn read_scalar<R: Read + Seek>(reader: &mut R, kind: u32) -> io::Result<Option<Value>> {
    let value = match kind {
        0 => Value::from(read_u8(reader)?),
        1 => Value::from(read_i8(reader)?),
        2 => Value::from(read_u16(reader)?),
        3 => Value::from(read_i16(reader)?),
        4 => Value::from(read_u32(reader)?),
        5 => Value::from(read_i32(reader)?),
        6 => {
            let number = f32::from_le_bytes(read_array::<4, _>(reader)?);
            serde_json::Number::from_f64(number as f64)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        7 => Value::Bool(read_u8(reader)? != 0),
        8 => Value::String(read_string(reader, MAX_STRING_BYTES)?),
        10 => Value::from(read_u64(reader)?),
        11 => Value::from(read_i64(reader)?),
        12 => {
            let number = f64::from_le_bytes(read_array::<8, _>(reader)?);
            serde_json::Number::from_f64(number)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        _ => {
            skip_value(reader, kind, 0)?;
            return Ok(None);
        }
    };
    Ok(Some(value))
}

fn read_u8<R: Read>(reader: &mut R) -> io::Result<u8> {
    let mut value = [0; 1];
    reader.read_exact(&mut value)?;
    Ok(value[0])
}
fn read_i8<R: Read>(reader: &mut R) -> io::Result<i8> {
    Ok(read_u8(reader)? as i8)
}
fn read_u16<R: Read>(reader: &mut R) -> io::Result<u16> {
    let mut value = [0; 2];
    reader.read_exact(&mut value)?;
    Ok(u16::from_le_bytes(value))
}
fn read_i16<R: Read>(reader: &mut R) -> io::Result<i16> {
    Ok(read_u16(reader)? as i16)
}
fn read_i32<R: Read>(reader: &mut R) -> io::Result<i32> {
    Ok(read_u32(reader)? as i32)
}
fn read_i64<R: Read>(reader: &mut R) -> io::Result<i64> {
    let mut value = [0; 8];
    reader.read_exact(&mut value)?;
    Ok(i64::from_le_bytes(value))
}
fn read_array<const N: usize, R: Read>(reader: &mut R) -> io::Result<[u8; N]> {
    let mut value = [0; N];
    reader.read_exact(&mut value)?;
    Ok(value)
}

fn as_usize(value: Option<&Value>) -> Option<usize> {
    value?
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
}

fn quantization_label(file_type: Option<u32>) -> Option<String> {
    // Values follow llama.cpp's public llama_ftype enum. They describe the
    // dominant model file type, not the quantization of every individual tensor.
    let value = file_type?;
    let label = match value {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4_MOE",
        39 => "NVFP4",
        40 => "Q1_0",
        41 => "Q2_0",
        42 => "F8_E4M3",
        _ => return Some(format!("GGUF file type {value}")),
    };
    Some(label.into())
}

pub fn inspect(path: &str) -> Result<ModelMetadata, String> {
    let model_path = Path::new(path);
    if model_path
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("gguf"))
    {
        return Err("The selected file must have a .gguf extension.".into());
    }
    let file =
        File::open(model_path).map_err(|error| format!("Could not open GGUF model: {error}"))?;
    let file_size_bytes = file.metadata().map_err(|error| error.to_string())?.len();
    let mut reader = BufReader::new(file);
    let mut magic = [0; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|error| format!("Could not read GGUF header: {error}"))?;
    if &magic != b"GGUF" {
        return Err("The file does not contain a GGUF header.".into());
    }
    let version = read_u32(&mut reader).map_err(|error| error.to_string())?;
    if !(2..=3).contains(&version) {
        return Err(format!(
            "GGUF version {version} is not supported by this metadata reader (expected v2 or v3)."
        ));
    }
    let tensor_count = read_u64(&mut reader).map_err(|error| error.to_string())?;
    if tensor_count == 0 {
        return Err("The GGUF file does not contain any model tensors.".into());
    }
    let metadata_count = read_u64(&mut reader).map_err(|error| error.to_string())?;
    if metadata_count > MAX_METADATA_ENTRIES {
        return Err("GGUF metadata entry count exceeds the parser safety limit.".into());
    }
    if tensor_count > 10_000_000 {
        return Err("GGUF tensor count exceeds the parser safety limit.".into());
    }

    let mut metadata = std::collections::HashMap::<String, Value>::new();
    for _ in 0..metadata_count {
        let key = read_string(&mut reader, MAX_KEY_BYTES).map_err(|error| error.to_string())?;
        let kind = read_u32(&mut reader).map_err(|error| error.to_string())?;
        let is_relevant = matches!(key.as_str(), "general.architecture" | "general.file_type")
            || key.ends_with(".block_count")
            || key.ends_with(".context_length")
            || key.ends_with(".embedding_length")
            || key.ends_with(".attention.head_count")
            || key.ends_with(".attention.head_count_kv")
            || key.ends_with(".attention.key_length")
            || key.ends_with(".attention.value_length");
        if is_relevant {
            if let Some(value) = read_scalar(&mut reader, kind)
                .map_err(|error| format!("Invalid GGUF metadata for {key}: {error}"))?
            {
                metadata.insert(key, value);
            }
        } else {
            skip_value(&mut reader, kind, 0)
                .map_err(|error| format!("Invalid GGUF metadata: {error}"))?;
        }
    }

    let architecture = metadata
        .get("general.architecture")
        .and_then(Value::as_str)
        .map(str::to_string);
    let arch_key = |suffix: &str| architecture.as_ref().map(|arch| format!("{arch}.{suffix}"));
    let get = |suffix: &str| arch_key(suffix).and_then(|key| metadata.get(&key));
    let layer_count = as_usize(get("block_count"));
    let context_length = as_usize(get("context_length"));
    let embedding_length = as_usize(get("embedding_length"));
    let attention_heads = as_usize(get("attention.head_count"));
    let key_value_heads = as_usize(get("attention.head_count_kv")).or(attention_heads);
    let key_length = as_usize(get("attention.key_length")).or_else(|| {
        embedding_length
            .zip(attention_heads)
            .and_then(|(embedding, heads)| embedding.checked_div(heads))
    });
    let value_length = as_usize(get("attention.value_length")).or(key_length);
    let file_type = metadata
        .get("general.file_type")
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok());
    let kv_bytes_per_token = layer_count
        .zip(key_value_heads)
        .zip(key_length.zip(value_length))
        .and_then(|((layers, kv_heads), (key_dim, value_dim))| {
            (layers as u64)
                .checked_mul(kv_heads as u64)?
                .checked_mul(key_dim as u64 + value_dim as u64)?
                .checked_mul(2)
        });

    Ok(ModelMetadata {
        path: model_path
            .canonicalize()
            .unwrap_or_else(|_| model_path.to_path_buf())
            .to_string_lossy()
            .to_string(),
        file_size_bytes,
        gguf_version: version,
        tensor_count,
        architecture,
        layer_count,
        context_length,
        embedding_length,
        attention_heads,
        key_value_heads,
        key_length,
        value_length,
        file_type,
        quantization: quantization_label(file_type),
        kv_bytes_per_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn put_u32(writer: &mut Vec<u8>, value: u32) {
        writer.extend(value.to_le_bytes());
    }
    fn put_u64(writer: &mut Vec<u8>, value: u64) {
        writer.extend(value.to_le_bytes());
    }
    fn put_string(writer: &mut Vec<u8>, value: &str) {
        put_u64(writer, value.len() as u64);
        writer.extend(value.as_bytes());
    }

    #[test]
    fn maps_llama_cpp_file_type_values_to_the_correct_labels() {
        assert_eq!(quantization_label(Some(7)).as_deref(), Some("Q8_0"));
        assert_eq!(quantization_label(Some(8)).as_deref(), Some("Q5_0"));
        assert_eq!(quantization_label(Some(9)).as_deref(), Some("Q5_1"));
        assert_eq!(quantization_label(Some(15)).as_deref(), Some("Q4_K_M"));
        assert_eq!(quantization_label(Some(19)).as_deref(), Some("IQ2_XXS"));
        assert_eq!(quantization_label(Some(32)).as_deref(), Some("BF16"));
        assert_eq!(quantization_label(Some(38)).as_deref(), Some("MXFP4_MOE"));
        assert_eq!(quantization_label(Some(42)).as_deref(), Some("F8_E4M3"));
    }

    #[test]
    fn reads_architecture_metadata_and_derives_kv_footprint() {
        let mut bytes = Vec::new();
        bytes.extend(b"GGUF");
        put_u32(&mut bytes, 3);
        put_u64(&mut bytes, 4);
        put_u64(&mut bytes, 9);

        put_string(&mut bytes, "general.architecture");
        put_u32(&mut bytes, 8);
        put_string(&mut bytes, "llama");
        for (key, value) in [
            ("general.file_type", 15),
            ("llama.block_count", 32),
            ("llama.context_length", 32768),
            ("llama.embedding_length", 4096),
            ("llama.attention.head_count", 32),
            ("llama.attention.head_count_kv", 8),
            ("llama.attention.key_length", 128),
            ("llama.attention.value_length", 128),
        ] {
            put_string(&mut bytes, key);
            put_u32(&mut bytes, 4);
            put_u32(&mut bytes, value);
        }

        let path =
            std::env::temp_dir().join(format!("spindle-gguf-test-{}.gguf", std::process::id()));
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        drop(file);
        let result = inspect(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(result.architecture.as_deref(), Some("llama"));
        assert_eq!(result.layer_count, Some(32));
        assert_eq!(result.context_length, Some(32768));
        assert_eq!(result.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(result.kv_bytes_per_token, Some(131_072));
    }

    #[test]
    fn rejects_metadata_values_that_extend_past_end_of_file() {
        let mut bytes = Vec::new();
        bytes.extend(b"GGUF");
        put_u32(&mut bytes, 3);
        put_u64(&mut bytes, 0);
        put_u64(&mut bytes, 1);
        put_string(&mut bytes, "general.description");
        put_u32(&mut bytes, 8);
        put_u64(&mut bytes, 4096);

        let path = std::env::temp_dir().join(format!(
            "spindle-gguf-truncated-{}.gguf",
            std::process::id()
        ));
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        drop(file);
        let error = inspect(path.to_str().unwrap()).unwrap_err();
        let _ = std::fs::remove_file(&path);

        assert!(error.contains("beyond the file"));
    }

    #[test]
    fn rejects_gguf_files_without_model_tensors() {
        let mut bytes = Vec::new();
        bytes.extend(b"GGUF");
        put_u32(&mut bytes, 3);
        put_u64(&mut bytes, 0);
        put_u64(&mut bytes, 0);

        let path = std::env::temp_dir().join(format!(
            "spindle-gguf-no-tensors-{}.gguf",
            std::process::id()
        ));
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        drop(file);
        let error = inspect(path.to_str().unwrap()).unwrap_err();
        let _ = std::fs::remove_file(&path);

        assert!(error.contains("does not contain any model tensors"));
    }

    #[test]
    fn rejects_non_gguf_input() {
        let error = inspect("not-a-model.bin").unwrap_err();
        assert!(error.contains(".gguf"));
    }
}
