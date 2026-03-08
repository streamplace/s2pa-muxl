/// Utilities for working with MP4 sample table (stbl) data.

/// Resolve the sample_description_index for every sample from the stsc table.
///
/// The stsc table uses a run-length style encoding: each entry says "starting at
/// first_chunk, each chunk has N samples with description index D". We expand
/// this to per-sample values. In canonical form each chunk has 1 sample,
/// but input files may have arbitrary chunk layouts.
pub fn resolve_sample_description_indices(
    stsc_entries: &[mp4::StscEntry],
    sample_count: u32,
) -> Vec<u32> {
    if stsc_entries.is_empty() || sample_count == 0 {
        return vec![1; sample_count as usize];
    }

    let mut result = Vec::with_capacity(sample_count as usize);
    let mut sample_idx = 0u32;

    for (i, entry) in stsc_entries.iter().enumerate() {
        let next_first_chunk = if i + 1 < stsc_entries.len() {
            stsc_entries[i + 1].first_chunk
        } else {
            let remaining = sample_count - sample_idx;
            let chunks_needed =
                (remaining + entry.samples_per_chunk - 1) / entry.samples_per_chunk;
            entry.first_chunk + chunks_needed
        };

        for _chunk in entry.first_chunk..next_first_chunk {
            for _s in 0..entry.samples_per_chunk {
                if sample_idx >= sample_count {
                    return result;
                }
                result.push(entry.sample_description_index);
                sample_idx += 1;
            }
        }
    }

    result
}

/// Build canonical stsc entries from per-sample description indices.
///
/// In canonical form, each chunk has 1 sample. We emit a new stsc entry
/// whenever the sample_description_index changes.
pub fn build_canonical_stsc(sample_desc_indices: &[u32]) -> Vec<mp4::StscEntry> {
    let mut entries = Vec::new();
    let mut current_desc_idx = 0u32;
    let mut first_sample_in_run = 1u32;

    for (i, &desc_idx) in sample_desc_indices.iter().enumerate() {
        let sample_num = (i as u32) + 1;
        if desc_idx != current_desc_idx {
            if current_desc_idx != 0 {
                entries.push(mp4::StscEntry {
                    first_chunk: first_sample_in_run,
                    samples_per_chunk: 1,
                    sample_description_index: current_desc_idx,
                    first_sample: first_sample_in_run,
                });
            }
            current_desc_idx = desc_idx;
            first_sample_in_run = sample_num;
        }
    }

    if current_desc_idx != 0 {
        entries.push(mp4::StscEntry {
            first_chunk: first_sample_in_run,
            samples_per_chunk: 1,
            sample_description_index: current_desc_idx,
            first_sample: first_sample_in_run,
        });
    }

    entries
}

/// Expand run-length encoded stts entries into per-sample durations.
pub fn expand_stts(stts_entries: &[mp4::SttsEntry], sample_count: u32) -> Vec<u32> {
    let mut durations = Vec::with_capacity(sample_count as usize);
    for entry in stts_entries {
        for _ in 0..entry.sample_count {
            durations.push(entry.sample_delta);
        }
    }
    durations.truncate(sample_count as usize);
    durations
}

/// Expand ctts entries into per-sample composition time offsets.
pub fn expand_ctts(ctts_entries: &[mp4::CttsEntry], sample_count: u32) -> Vec<i32> {
    let mut offsets = Vec::with_capacity(sample_count as usize);
    for entry in ctts_entries {
        for _ in 0..entry.sample_count {
            offsets.push(entry.sample_offset);
        }
    }
    offsets.truncate(sample_count as usize);
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_single_entry() {
        let entries = vec![mp4::StscEntry {
            first_chunk: 1,
            samples_per_chunk: 1,
            sample_description_index: 1,
            first_sample: 1,
        }];
        let result = resolve_sample_description_indices(&entries, 5);
        assert_eq!(result, vec![1, 1, 1, 1, 1]);
    }

    #[test]
    fn test_resolve_empty() {
        let result = resolve_sample_description_indices(&[], 3);
        assert_eq!(result, vec![1, 1, 1]);
    }

    #[test]
    fn test_build_canonical_stsc_single() {
        let indices = vec![1, 1, 1, 1, 1];
        let entries = build_canonical_stsc(&indices);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].first_chunk, 1);
        assert_eq!(entries[0].sample_description_index, 1);
    }

    #[test]
    fn test_build_canonical_stsc_transition() {
        let indices = vec![1, 1, 1, 2, 2];
        let entries = build_canonical_stsc(&indices);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].first_chunk, 1);
        assert_eq!(entries[0].sample_description_index, 1);
        assert_eq!(entries[1].first_chunk, 4);
        assert_eq!(entries[1].sample_description_index, 2);
    }

    #[test]
    fn test_expand_stts() {
        let entries = vec![
            mp4::SttsEntry { sample_count: 3, sample_delta: 1000 },
            mp4::SttsEntry { sample_count: 2, sample_delta: 2000 },
        ];
        assert_eq!(expand_stts(&entries, 5), vec![1000, 1000, 1000, 2000, 2000]);
    }
}
