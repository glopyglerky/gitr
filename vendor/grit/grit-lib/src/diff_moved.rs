//! Move detection for `git diff --color-moved`, implementing the same behavior
//! as git's `add_lines_to_move_detection` + `mark_color_as_moved` + `dim_moved_lines`.
//!
//! Operates on the already-rendered *plain* unified diff text (one or more files
//! of `diff --git` headers, `@@` hunk headers, and `+`/`-`/context body lines).
//! It identifies blocks of moved code and returns, for every line of the input
//! patch, a [`MovedClass`] telling the colorizer which color slot to use.

/// `--color-moved` mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMovedMode {
    /// No move detection.
    No,
    /// `plain`: all moved lines use the moved color, no alternation.
    Plain,
    /// `blocks`: alternate-by-block detection but no alternate color.
    Blocks,
    /// `zebra`: alternate colors between adjacent moved blocks.
    Zebra,
    /// `dimmed-zebra`: like zebra but dim the interior of blocks.
    ZebraDim,
}

impl ColorMovedMode {
    /// Parse a `--color-moved[=<mode>]` value. `default` maps to `Zebra`.
    /// Returns `None` for an unrecognized mode.
    #[must_use]
    pub fn parse(arg: &str) -> Option<Self> {
        match arg {
            "no" | "false" | "off" => Some(Self::No),
            "plain" => Some(Self::Plain),
            "blocks" => Some(Self::Blocks),
            "zebra" | "default" | "true" | "on" => Some(Self::Zebra),
            "dimmed-zebra" | "dimmed_zebra" => Some(Self::ZebraDim),
            _ => None,
        }
    }
}

// Whitespace handling bits for move detection (mirror git's XDF_* + the
// allow-indentation-change bit).
pub const MOVED_WS_IGNORE_ALL_SPACE: u32 = 1;
pub const MOVED_WS_IGNORE_SPACE_CHANGE: u32 = 2;
pub const MOVED_WS_IGNORE_SPACE_AT_EOL: u32 = 4;
pub const MOVED_WS_ALLOW_INDENTATION_CHANGE: u32 = 8;
pub const MOVED_WS_ERROR: u32 = 16;

const WS_FLAGS_MASK: u32 =
    MOVED_WS_IGNORE_ALL_SPACE | MOVED_WS_IGNORE_SPACE_CHANGE | MOVED_WS_IGNORE_SPACE_AT_EOL;

/// Parse a comma-separated `--color-moved-ws=<modes>` value into the bit flags.
/// Sets [`MOVED_WS_ERROR`] for unknown modes or the illegal
/// allow-indentation-change + other-ws combination (git errors out then).
#[must_use]
pub fn parse_color_moved_ws(arg: &str) -> u32 {
    let mut ret = 0u32;
    for raw in arg.split(',') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        match part {
            "no" => ret = 0,
            "ignore-space-change" => ret |= MOVED_WS_IGNORE_SPACE_CHANGE,
            "ignore-space-at-eol" => ret |= MOVED_WS_IGNORE_SPACE_AT_EOL,
            "ignore-all-space" => ret |= MOVED_WS_IGNORE_ALL_SPACE,
            "allow-indentation-change" => ret |= MOVED_WS_ALLOW_INDENTATION_CHANGE,
            _ => ret |= MOVED_WS_ERROR,
        }
    }
    if (ret & MOVED_WS_ALLOW_INDENTATION_CHANGE) != 0 && (ret & WS_FLAGS_MASK) != 0 {
        ret |= MOVED_WS_ERROR;
    }
    ret
}

/// The color class assigned to one emitted line after move detection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MovedClass {
    /// Not a moved line: use the normal old/new/context color.
    None,
    /// Moved, primary color (`oldMoved` / `newMoved`).
    Moved,
    /// Moved, alternate color (`oldMovedAlternative` / `newMovedAlternative`).
    MovedAlt,
    /// Moved, primary dim (`oldMovedDimmed` / `newMovedDimmed`).
    MovedDim,
    /// Moved, alternate dim (`oldMovedAlternativeDimmed` / ...).
    MovedAltDim,
}

const INDENT_BLANKLINE: i64 = i64::MIN;

#[derive(Clone)]
struct Symbol {
    /// Index into the original patch-line list.
    line_index: usize,
    /// `+` (true) or `-` (false).
    is_plus: bool,
    /// Raw body of the line (without the leading +/-, with trailing newline stripped).
    body: String,
    /// Interned content id (whitespace-normalized per ws flags).
    id: usize,
    /// Visual indent width, or [`INDENT_BLANKLINE`] for blank lines.
    indent_width: i64,
    /// Resulting move class.
    class: MovedClass,
    /// Whether this line is part of a moved block at all.
    moved: bool,
    /// Whether the alternate flag is set.
    alt: bool,
    /// Whether the line was dimmed (uninteresting interior).
    uninteresting: bool,
}

/// Normalize a line for interning / comparison according to the ws flags
/// (mirrors xdiff_compare_lines / xdiff_hash_string semantics enough for move
/// detection: the comparison only needs a canonical key per ws mode).
fn normalize_key(body: &str, flags: u32) -> String {
    let ws = flags & WS_FLAGS_MASK;
    if ws == 0 {
        return body.to_owned();
    }
    if ws & MOVED_WS_IGNORE_ALL_SPACE != 0 {
        return body.chars().filter(|c| !c.is_whitespace()).collect();
    }
    if ws & MOVED_WS_IGNORE_SPACE_CHANGE != 0 {
        // Collapse runs of whitespace to a single space and trim trailing ws.
        let mut out = String::with_capacity(body.len());
        let mut in_ws = false;
        for c in body.chars() {
            if c.is_whitespace() {
                in_ws = true;
            } else {
                if in_ws && !out.is_empty() {
                    out.push(' ');
                }
                in_ws = false;
                out.push(c);
            }
        }
        return out;
    }
    if ws & MOVED_WS_IGNORE_SPACE_AT_EOL != 0 {
        return body.trim_end().to_owned();
    }
    body.to_owned()
}

/// Visual indent width + blankness, mirroring git's `fill_es_indent_data`
/// (8-wide tab stops, like git's default `WS_TAB_WIDTH`).
fn fill_indent(body: &str) -> (usize, i64) {
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut off = 0usize;
    // skip \f \v \r at start of indentation
    while off < len
        && (bytes[off] == b'\x0c'
            || bytes[off] == b'\x0b'
            || (off + 1 < len && bytes[off] == b'\r'))
    {
        off += 1;
    }
    let tab_width = 8i64;
    let mut width = 0i64;
    loop {
        if off < len && bytes[off] == b' ' {
            width += 1;
            off += 1;
        } else if off < len && bytes[off] == b'\t' {
            width += tab_width - (width % tab_width);
            off += 1;
            while off < len && bytes[off] == b'\t' {
                width += tab_width;
                off += 1;
            }
        } else {
            break;
        }
    }
    // is the line blank?
    let mut i = off;
    while i < len && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    if i == len {
        (len, INDENT_BLANKLINE)
    } else {
        (off, width)
    }
}

struct MovedBlock {
    /// Index into `syms` of the matched entry on the previous line of this block.
    match_idx: usize,
    wsd: i64,
}

/// Run move detection over a plain multi-file unified diff. Returns a vector
/// parallel to the patch's lines (`patch.split_inclusive('\n')`) giving each
/// line's [`MovedClass`]; non-+/- lines are always [`MovedClass::None`].
#[must_use]
pub fn detect_moved_lines(patch: &str, mode: ColorMovedMode, ws_flags: u32) -> Vec<MovedClass> {
    let lines: Vec<&str> = patch.split_inclusive('\n').collect();
    let mut classes = vec![MovedClass::None; lines.len()];
    if mode == ColorMovedMode::No {
        return classes;
    }
    let allow_indent = ws_flags & MOVED_WS_ALLOW_INDENTATION_CHANGE != 0;

    // Build the emitted-symbol list: only +/- body lines inside hunks, resetting
    // at any non +/- line (so consecutive runs map to next_line chains).
    let mut syms: Vec<Symbol> = Vec::new();
    let mut in_hunk = false;
    let mut interner: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut next_id = 0usize;
    for (idx, &raw) in lines.iter().enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if line.starts_with("@@") {
            in_hunk = true;
            continue;
        }
        if line.starts_with("diff --git")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("index ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity")
            || line.starts_with("dissimilarity")
            || line.starts_with("rename ")
            || line.starts_with("copy ")
            || line.starts_with("Binary files")
            || line.starts_with('\\')
        {
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            continue;
        }
        let (is_plus, body) = if let Some(rest) = line.strip_prefix('+') {
            (true, rest)
        } else if let Some(rest) = line.strip_prefix('-') {
            (false, rest)
        } else {
            // context (or blank line rendered as a bare newline)
            continue;
        };

        let (indent_off, indent_width) = if allow_indent {
            fill_indent(body)
        } else {
            (0, 0)
        };
        // With allow-indentation-change, Git interns/compares from the indent
        // offset (`l->line + l->indent_off`), so leading indentation does not
        // affect the content id; the indent delta is checked separately.
        let key = if allow_indent {
            normalize_key(&body[indent_off.min(body.len())..], ws_flags)
        } else {
            normalize_key(body, ws_flags)
        };
        let id = *interner.entry(key).or_insert_with(|| {
            let v = next_id;
            next_id += 1;
            v
        });
        syms.push(Symbol {
            line_index: idx,
            is_plus,
            body: body.to_owned(),
            id,
            indent_width,
            class: MovedClass::None,
            moved: false,
            alt: false,
            uninteresting: false,
        });
    }

    if syms.is_empty() {
        return classes;
    }

    // next_line chains: for each symbol, the next symbol of the same sign that is
    // immediately contiguous in the emitted stream. We approximate git's
    // prev_line linkage by detecting contiguity through line_index adjacency
    // within the same +/- run.
    let n = syms.len();
    let mut next_line: Vec<Option<usize>> = vec![None; n];
    for i in 1..n {
        // Contiguous if the previous symbol's patch line is immediately before
        // this one and has the same sign (git resets prev_line on any non +/-).
        if syms[i].line_index == syms[i - 1].line_index + 1
            && syms[i].is_plus == syms[i - 1].is_plus
        {
            next_line[i - 1] = Some(i);
        }
    }

    // Match lists per id: add[id] / del[id] hold symbol indices (in order, but we
    // build the head-insertion lists like git so iteration matches).
    let mut add_head: Vec<Option<usize>> = vec![None; next_id];
    let mut del_head: Vec<Option<usize>> = vec![None; next_id];
    let mut next_match: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let id = syms[i].id;
        if syms[i].is_plus {
            next_match[i] = add_head[id];
            add_head[id] = Some(i);
        } else {
            next_match[i] = del_head[id];
            del_head[id] = Some(i);
        }
    }
    // git inserts at head, so the first-added ends up last; iterating next_match
    // from head visits most-recent-first. git builds the same way, so keep it.

    let min_alnum = 20usize; // COLOR_MOVED_MIN_ALNUM_COUNT

    // adjust_last_block: returns whether the block (the last `block_length`
    // symbols ending just before position `pos`) has enough alnum chars; if not,
    // clears the moved/alt flags on those lines.
    let adjust_last_block = |syms: &mut [Symbol], pos: usize, block_length: usize| -> bool {
        if mode == ColorMovedMode::Plain {
            return block_length != 0;
        }
        if block_length == 0 {
            return false;
        }
        let mut alnum = 0usize;
        for i in 1..=block_length {
            let s = &syms[pos - i];
            for c in s.body.chars() {
                if c.is_alphanumeric() {
                    alnum += 1;
                    if alnum >= min_alnum {
                        return true;
                    }
                }
            }
        }
        for i in 1..=block_length {
            syms[pos - i].moved = false;
            syms[pos - i].alt = false;
        }
        false
    };

    // cmp_in_block_with_wsd: 0 == match, 1 == no match.
    let cmp_in_block = |cur: &Symbol, l: &Symbol, pmb_wsd: &mut i64| -> bool {
        if cur.id != l.id {
            return false;
        }
        let a_width = cur.indent_width;
        if a_width == INDENT_BLANKLINE {
            return true;
        }
        let delta = l.indent_width - a_width;
        if *pmb_wsd == INDENT_BLANKLINE {
            *pmb_wsd = delta;
        }
        delta == *pmb_wsd
    };

    // pmb_advance_or_null: advance each potential block to its next line if it
    // still matches; drop the rest.
    let pmb_advance = |syms: &[Symbol], pmb: &mut Vec<MovedBlock>, l_idx: usize| {
        let l = &syms[l_idx];
        let mut kept: Vec<MovedBlock> = Vec::with_capacity(pmb.len());
        for b in pmb.iter() {
            let cur = next_line[b.match_idx];
            let matched = if let Some(cur_idx) = cur {
                if allow_indent {
                    let mut wsd = b.wsd;
                    let m = cmp_in_block(&syms[cur_idx], l, &mut wsd);
                    if m {
                        kept.push(MovedBlock {
                            match_idx: cur_idx,
                            wsd,
                        });
                    }
                    m
                } else {
                    let m = syms[cur_idx].id == l.id;
                    if m {
                        kept.push(MovedBlock {
                            match_idx: cur_idx,
                            wsd: b.wsd,
                        });
                    }
                    m
                }
            } else {
                false
            };
            let _ = matched;
        }
        *pmb = kept;
    };

    // mark_color_as_moved: matches git's behavior. We emulate git's
    // `for (n = 0; n < nr; n++)` loop: a "rewind" sets `nn -= block_length`
    // here and the loop epilogue still does `nn += 1`, giving git's net
    // `nn -= block_length - 1` (re-examine from the block's 2nd line).
    let mut pmb: Vec<MovedBlock> = Vec::new();
    let mut flipped_block = 0i32;
    let mut block_length = 0usize;
    // moved_symbol: Some(true)=plus, Some(false)=minus, None=neither
    let mut moved_symbol: Option<bool> = None;

    let mut nn = 0usize;
    // Safety cap: the rewind logic strictly shrinks the re-examined block each
    // time, so this terminates, but guard against any pathological input causing
    // an unbounded loop (rewinds re-visit lines, so bound generously).
    let max_iters = n.saturating_mul(n).saturating_add(n).saturating_add(16);
    let mut iters = 0usize;
    // True once a barrier (a non-`+/-` line, i.e. a gap in line_index, or the
    // very start) has been processed for the line at `nn`. Git resets
    // `flipped_block` on every non-`+/-` symbol (`switch ... default`), which our
    // `+/-`-only `syms` list must emulate at line_index gaps.
    while nn < n {
        iters += 1;
        if iters > max_iters {
            break;
        }
        // Emulate git's non-`+/-` line handling: when a context line separated
        // this symbol from the previous one (gap in line_index), reset
        // `flipped_block` and finalize any open block before processing.
        let barrier = nn == 0 || syms[nn].line_index != syms[nn - 1].line_index + 1;
        if barrier {
            if !pmb.is_empty() {
                adjust_last_block(&mut syms, nn, block_length);
                pmb.clear();
                block_length = 0;
            }
            flipped_block = 0;
            moved_symbol = None;
        }
        // match for this line: a plus matches dels, a minus matches adds.
        let mut match_idx: Option<usize> = if syms[nn].is_plus {
            del_head[syms[nn].id]
        } else {
            add_head[syms[nn].id]
        };

        let cur_sym = syms[nn].is_plus;
        if !pmb.is_empty() && (match_idx.is_none() || Some(cur_sym) != moved_symbol) {
            if !adjust_last_block(&mut syms, nn, block_length) && block_length > 1 {
                match_idx = None;
                nn -= block_length;
            }
            pmb.clear();
            block_length = 0;
            flipped_block = 0;
        }

        let Some(match_start) = match_idx else {
            moved_symbol = None;
            nn += 1;
            continue;
        };

        if mode == ColorMovedMode::Plain {
            syms[nn].moved = true;
            nn += 1;
            continue;
        }

        pmb_advance(&syms, &mut pmb, nn);

        if pmb.is_empty() {
            let contiguous = adjust_last_block(&mut syms, nn, block_length);

            if !contiguous && block_length > 1 {
                // Nothing carried over: back the cursor up to the block start so
                // a run that begins on its second line can still be picked up.
                // pmb is left empty here (no refill).
                nn -= block_length;
            } else {
                // fill_potential_moved_blocks: set up pmb from all matches.
                let mut m = Some(match_start);
                pmb.clear();
                while let Some(mi) = m {
                    let wsd = if allow_indent {
                        compute_ws_delta(&syms[nn], &syms[mi])
                    } else {
                        0
                    };
                    pmb.push(MovedBlock { match_idx: mi, wsd });
                    m = next_match[mi];
                }
            }

            if contiguous && !pmb.is_empty() && moved_symbol == Some(syms[nn].is_plus) {
                flipped_block = (flipped_block + 1) % 2;
            } else {
                flipped_block = 0;
            }

            moved_symbol = if !pmb.is_empty() {
                Some(syms[nn].is_plus)
            } else {
                None
            };

            block_length = 0;
        }

        if !pmb.is_empty() {
            block_length += 1;
            syms[nn].moved = true;
            if flipped_block != 0 && mode != ColorMovedMode::Blocks {
                syms[nn].alt = true;
            }
        }

        nn += 1;
    }
    adjust_last_block(&mut syms, n, block_length);

    // dim_moved_lines (dimmed-zebra only)
    if mode == ColorMovedMode::ZebraDim {
        dim_moved_lines(&mut syms);
    }

    // Resolve classes.
    for s in &mut syms {
        if !s.moved {
            s.class = MovedClass::None;
        } else {
            s.class = match (s.alt, s.uninteresting) {
                (true, true) => MovedClass::MovedAltDim,
                (true, false) => MovedClass::MovedAlt,
                (false, true) => MovedClass::MovedDim,
                (false, false) => MovedClass::Moved,
            };
        }
    }

    for s in &syms {
        classes[s.line_index] = s.class;
    }
    classes
}

fn compute_ws_delta(a: &Symbol, b: &Symbol) -> i64 {
    if a.indent_width == INDENT_BLANKLINE && b.indent_width == INDENT_BLANKLINE {
        return INDENT_BLANKLINE;
    }
    a.indent_width - b.indent_width
}

fn dim_moved_lines(syms: &mut [Symbol]) {
    let n = syms.len();
    for i in 0..n {
        if !syms[i].moved {
            continue;
        }
        // prev/next are only "real" if they are +/- (they always are in syms,
        // but a gap in line_index means a context line separated them).
        let prev = if i > 0 && syms[i - 1].line_index + 1 == syms[i].line_index {
            Some(i - 1)
        } else {
            None
        };
        let next = if i + 1 < n && syms[i].line_index + 1 == syms[i + 1].line_index {
            Some(i + 1)
        } else {
            None
        };

        let zebra_mask = |s: &Symbol| -> (bool, bool) { (s.moved, s.alt) };
        let cur_mask = zebra_mask(&syms[i]);

        // Inside a block? prev and next share the same moved+alt mask.
        if let (Some(p), Some(nx)) = (prev, next) {
            if zebra_mask(&syms[p]) == cur_mask && zebra_mask(&syms[nx]) == cur_mask {
                syms[i].uninteresting = true;
                continue;
            }
        }
        // Interesting bound checks.
        if let Some(p) = prev {
            if syms[p].moved && syms[p].alt != syms[i].alt {
                continue;
            }
        }
        if let Some(nx) = next {
            if syms[nx].moved && syms[nx].alt != syms[i].alt {
                continue;
            }
        }
        syms[i].uninteresting = true;
    }
}
