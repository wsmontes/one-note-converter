//! Microsoft Cabinet reader, enough for a OneNote package (`.onepkg`).
//!
//! A cabinet is `CFHEADER | CFFOLDER[] | CFFILE[] | CFDATA[]`. OneNote writes
//! either uncompressed folders (`typeCompress` 0) or MSZIP (1); Quantum and LZX
//! are rejected rather than guessed. MSZIP blocks are raw deflate streams that
//! each continue the previous block's 32 KiB history, so the decompressor is
//! re-created per block with that history as its preset dictionary.

use flate2::{Decompress, FlushDecompress};

/// One `CFFILE` entry.
#[derive(Debug, Clone)]
pub struct CabEntry {
    pub name: String,
    pub size: usize,
    pub folder: usize,
    pub offset: usize,
}

/// A parsed cabinet; folder payloads are decompressed on demand.
#[derive(Debug, Clone, Default)]
pub struct Cabinet {
    pub entries: Vec<CabEntry>,
    payloads: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct Folder {
    data_start: usize,
    blocks: usize,
    compression: u16,
    reserve: usize,
}

impl Cabinet {
    pub fn parse(data: &[u8]) -> Result<Cabinet, String> {
        if data.len() < 36 || &data[..4] != b"MSCF" {
            return Err("not a cabinet: missing MSCF signature".to_string());
        }
        let at = |offset: usize| -> Result<usize, String> {
            let bytes = data
                .get(offset..offset + 4)
                .ok_or_else(|| format!("cabinet truncated at {offset:#x}"))?;
            Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
        };
        let declared = at(8)?;
        if declared > data.len() {
            return Err(format!(
                "cbCabinet {declared} exceeds the file size {}",
                data.len()
            ));
        }
        let files_at = at(16)?;
        let folders = u16::from_le_bytes(data[26..28].try_into().unwrap()) as usize;
        let file_count = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
        let flags = u16::from_le_bytes(data[30..32].try_into().unwrap());

        // Optional per-structure reserved areas ([MS-CAB] 2.1, 2.4).
        let mut cursor = 36;
        let (data_reserve, folder_reserve) = if flags & 0x0004 != 0 {
            let header_reserve = u16::from_le_bytes(
                data.get(cursor..cursor + 2)
                    .ok_or("cabinet reserves truncated")?
                    .try_into()
                    .unwrap(),
            ) as usize;
            let folder = *data.get(cursor + 2).ok_or("cabinet reserves truncated")? as usize;
            let data_bytes = *data.get(cursor + 3).ok_or("cabinet reserves truncated")? as usize;
            cursor += 4 + header_reserve;
            (data_bytes, folder)
        } else {
            (0, 0)
        };

        let mut parsed_folders = Vec::new();
        for _ in 0..folders {
            let start = at(cursor)?;
            let blocks = u16::from_le_bytes(
                data.get(cursor + 4..cursor + 6)
                    .ok_or("CFFOLDER truncated")?
                    .try_into()
                    .unwrap(),
            ) as usize;
            let compression = u16::from_le_bytes(
                data.get(cursor + 6..cursor + 8)
                    .ok_or("CFFOLDER truncated")?
                    .try_into()
                    .unwrap(),
            ) & 0x000F;
            parsed_folders.push(Folder {
                data_start: start,
                blocks,
                compression,
                reserve: data_reserve,
            });
            cursor += 8 + folder_reserve;
        }

        let mut entries = Vec::new();
        cursor = files_at;
        for _ in 0..file_count {
            let size = at(cursor)?;
            let offset = at(cursor + 4)?;
            let folder = u16::from_le_bytes(
                data.get(cursor + 8..cursor + 10)
                    .ok_or("CFFILE truncated")?
                    .try_into()
                    .unwrap(),
            ) as usize;
            let name_at = cursor + 16;
            let end = data.get(name_at..)
                .ok_or("CFFILE name offset exceeds cabinet length")?
                .iter()
                .position(|byte| *byte == 0)
                .ok_or("CFFILE name is not terminated")?;
            let name = String::from_utf8_lossy(&data[name_at..name_at + end]).to_string();
            entries.push(CabEntry {
                name,
                size,
                folder,
                offset,
            });
            cursor = name_at + end + 1;
        }

        for entry in &entries {
            if entry.folder >= parsed_folders.len() {
                return Err(format!("{}: folder index out of range", entry.name));
            }
        }
        let folders = parsed_folders;
        // Decompression is verified when a payload is requested; keep the
        // folder table available by resolving sizes eagerly so callers can fail
        // before writing anything.
        let payloads = decompress_folders(data, &folders)?;
        let mut cabinet = Cabinet {
            entries,
            payloads: Vec::new(),
        };
        for entry in &mut cabinet.entries {
            let size = payloads[entry.folder].len();
            if entry.offset.saturating_add(entry.size) > size {
                return Err(format!(
                    "{}: {} bytes at {} exceed the {size} bytes of folder {}",
                    entry.name, entry.size, entry.offset, entry.folder
                ));
            }
        }
        cabinet.payloads = payloads;
        Ok(cabinet)
    }

    /// Bytes of one entry, decompressed.
    pub fn read(&self, index: usize) -> Option<&[u8]> {
        let entry = self.entries.get(index)?;
        let folder = self.payloads.get(entry.folder)?;
        folder.get(entry.offset..entry.offset + entry.size)
    }
}

fn decompress_folders(data: &[u8], folders: &[Folder]) -> Result<Vec<Vec<u8>>, String> {
    let mut payloads = Vec::new();
    for folder in folders {
        payloads.push(decompress_folder(data, folder)?);
    }
    Ok(payloads)
}

fn decompress_folder(data: &[u8], folder: &Folder) -> Result<Vec<u8>, String> {
    let mut cursor = folder.data_start;
    let mut out: Vec<u8> = Vec::new();
    for block in 0..folder.blocks {
        let cb_data = u16::from_le_bytes(
            data.get(cursor + 4..cursor + 6)
                .ok_or_else(|| format!("CFDATA {block} truncated"))?
                .try_into()
                .unwrap(),
        ) as usize;
        let cb_uncomp = u16::from_le_bytes(
            data.get(cursor + 6..cursor + 8)
                .ok_or_else(|| format!("CFDATA {block} truncated"))?
                .try_into()
                .unwrap(),
        ) as usize;
        let start = cursor + 8 + folder.reserve;
        let raw = data
            .get(start..start + cb_data)
            .ok_or_else(|| format!("CFDATA {block} payload truncated"))?;
        match folder.compression {
            0 => {
                if raw.len() != cb_uncomp {
                    return Err(format!("CFDATA {block}: stored size {} differs from declared {cb_uncomp}", raw.len()));
                }
                out.extend_from_slice(raw);
            },
            1 => {
                // MSZIP: each block is `CK` followed by a raw deflate stream.
                let body = raw
                    .strip_prefix(b"CK")
                    .ok_or_else(|| format!("CFDATA {block}: missing MSZIP signature"))?;
                let history_start = out.len().saturating_sub(32 * 1024);
                let history = out[history_start..].to_vec();
                let mut inflater = Decompress::new(false);
                if !history.is_empty() {
                    inflater
                        .set_dictionary(&history)
                        .map_err(|error| format!("CFDATA {block}: {error}"))?;
                }
                let mut block_out: Vec<u8> = Vec::with_capacity(cb_uncomp);
                let mut consumed = 0usize;
                loop {
                    let before_out = inflater.total_out();
                    let before_in = inflater.total_in();
                    let mut chunk = vec![0u8; 32 * 1024];
                    let status = inflater
                        .decompress(&body[consumed..], &mut chunk, FlushDecompress::None)
                        .map_err(|error| format!("CFDATA {block}: {error}"))?;
                    let produced = (inflater.total_out() - before_out) as usize;
                    chunk.truncate(produced);
                    if produced > cb_uncomp.saturating_sub(block_out.len()) {
                        return Err(format!("CFDATA {block}: decompression exceeds declared {cb_uncomp} bytes"));
                    }
                    block_out.extend_from_slice(&chunk);
                    consumed = inflater.total_in() as usize;
                    if status == flate2::Status::StreamEnd {
                        break;
                    }
                    if produced == 0 && inflater.total_in() == before_in {
                        return Err(format!("CFDATA {block}: inflater made no progress"));
                    }
                    if produced == 0 && consumed >= body.len() {
                        // The stream ended without zlib reporting StreamEnd; the
                        // declared uncompressed size is the authority below.
                        break;
                    }
                }
                if block_out.len() != cb_uncomp {
                    return Err(format!(
                        "CFDATA {block}: decoded {} of the declared {cb_uncomp} bytes",
                        block_out.len()
                    ));
                }
                out.extend_from_slice(&block_out);
            }
            other => {
                return Err(format!(
                    "folder compression {other} is not supported (MSZIP only)"
                ));
            }
        }
        cursor = start + cb_data;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression};

    /// Build a one-folder cabinet; `mszip` compresses each 32 KiB chunk as a
    /// `CK`-prefixed deflate stream that knows the previous chunk as dictionary.
    fn build(entries: &[(&str, Vec<u8>)], mszip: bool) -> Vec<u8> {
        let mut folder_payload = entries[0].1.clone();
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        let mut uncompressed: Vec<u16> = Vec::new();
        if mszip {
            let payload = &entries[0].1;
            let mut history: Vec<u8> = Vec::new();
            for chunk in payload.chunks(32 * 1024) {
                let mut encoder = Compress::new(Compression::default(), false);
                if !history.is_empty() {
                    encoder
                        .set_dictionary(&history)
                        .expect("dictionary accepted");
                }
                let mut body = Vec::with_capacity(chunk.len() + 1024);
                loop {
                    let status = encoder
                        .compress_vec(&chunk[0..], &mut body, flate2::FlushCompress::Finish)
                        .expect("compressed");
                    if status == flate2::Status::StreamEnd {
                        break;
                    }
                }
                let mut block = b"CK".to_vec();
                block.extend_from_slice(&body);
                blocks.push(block);
                uncompressed.push(chunk.len() as u16);
                history.extend_from_slice(chunk);
            }
        } else {
            blocks.push(folder_payload.clone());
            uncompressed.push(folder_payload.len() as u16);
        }
        let _ = &mut folder_payload;

        let header_len = 36;
        let folder_at = header_len;
        let files_at = folder_at + 8;
        let mut file_table: Vec<u8> = Vec::new();
        let mut offset = 0usize;
        for (name, bytes) in entries {
            file_table.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            file_table.extend_from_slice(&(offset as u32).to_le_bytes());
            file_table.extend_from_slice(&0u16.to_le_bytes()); // folder
            file_table.extend_from_slice(&0u16.to_le_bytes()); // date
            file_table.extend_from_slice(&0u16.to_le_bytes()); // time
            file_table.extend_from_slice(&0u16.to_le_bytes()); // attributes
            file_table.extend_from_slice(name.as_bytes());
            file_table.push(0);
            offset += bytes.len();
        }
        let mut data_at = files_at + file_table.len();
        let mut data: Vec<u8> = Vec::new();
        let mut block_count = 0u16;
        for block in &blocks {
            // `cbData` covers the block payload including the MSZIP `CK` prefix.
            let content_len = block.len();
            data.extend_from_slice(&0u32.to_le_bytes()); // checksum
            data.extend_from_slice(&(content_len as u16).to_le_bytes());
            data.extend_from_slice(&uncompressed[usize::from(block_count)].to_le_bytes());
            data.extend_from_slice(block);
            block_count += 1;
            let _ = &mut data_at;
        }
        let total = files_at + file_table.len() + data.len();

        let mut out = Vec::new();
        out.extend_from_slice(b"MSCF");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(files_at as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.push(3); // versionMinor
        out.push(1); // versionMajor
        out.extend_from_slice(&1u16.to_le_bytes()); // cFolders
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // setID
        out.extend_from_slice(&0u16.to_le_bytes()); // iCabinet
        debug_assert_eq!(out.len(), header_len);
        out.extend_from_slice(&((files_at + file_table.len()) as u32).to_le_bytes());
        out.extend_from_slice(&block_count.to_le_bytes());
        out.extend_from_slice(&(if mszip { 1u16 } else { 0u16 }).to_le_bytes());
        out.extend_from_slice(&file_table);
        out.extend_from_slice(&data);
        debug_assert_eq!(out.len(), total);
        out
    }

    #[test]
    fn uncompressed_entries_round_trip() {
        let data = build(&[("a.one", b"hello world".to_vec())], false);
        let cabinet = Cabinet::parse(&data).unwrap();
        assert_eq!(cabinet.entries.len(), 1);
        assert_eq!(cabinet.entries[0].name, "a.one");
        assert_eq!(cabinet.read(0).unwrap(), b"hello world");
    }

    #[test]
    fn mszip_blocks_continue_the_previous_history() {
        // Two chunks of the same byte: the second block only compresses well
        // (and only decodes correctly) when the reader presets the dictionary.
        let payload = vec![b'A'; 40 * 1024];
        let data = build(&[("big.one", payload.clone())], true);
        let cabinet = Cabinet::parse(&data).unwrap();
        let bytes = cabinet.read(0).unwrap();
        assert_eq!(bytes.len(), payload.len());
        assert!(bytes.iter().all(|byte| *byte == b'A'));
    }

    #[test]
    fn missing_signature_is_an_error() {
        assert!(Cabinet::parse(b"nope").is_err());
    }
}
