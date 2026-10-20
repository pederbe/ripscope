//! The private cscope text grammar, separate from filename/identifier EREs.
use super::{ByteSet, PatternError, ResourceLimit, emit_byte, emit_set};

#[derive(Default)]
struct Frame {
    start: usize,
    atom: Option<usize>,
    atom_depth: usize,
    depth: usize,
}

pub(super) fn translate(input: &[u8], depth_limit: usize) -> Result<String, PatternError> {
    let mut out = String::new();
    let mut frames = vec![Frame::default()];
    let mut p = 0;
    while p < input.len() {
        let byte = input[p];
        p += 1;
        match byte {
            b'(' => {
                if frames.len() > depth_limit {
                    return Err(PatternError::Resource(ResourceLimit::GroupDepth));
                }
                frames.push(Frame {
                    start: out.len(),
                    atom: None,
                    ..Frame::default()
                });
                out.push_str("(?:");
            }
            b')' => {
                if frames.len() == 1 {
                    return Err(PatternError::InvalidSyntax);
                }
                let frame = frames.pop().unwrap();
                if frame.atom.is_none() {
                    return Err(PatternError::InvalidSyntax);
                }
                out.push(')');
                let parent = frames.last_mut().unwrap();
                parent.atom = Some(frame.start);
                parent.atom_depth = frame.depth + 1;
                parent.depth = parent.depth.max(parent.atom_depth);
                if parent.depth > depth_limit {
                    return Err(PatternError::Resource(ResourceLimit::GroupDepth));
                }
            }
            b'|' => {
                // The reference accepts a single outer delimiter on either end.
                if frames.len() == 1 && (p == 1 || p == input.len()) {
                    if p == input.len() && frames[0].atom.is_none() {
                        return Err(PatternError::InvalidSyntax);
                    }
                    continue;
                }
                let frame = frames.last_mut().unwrap();
                if frame.atom.take().is_none() {
                    return Err(PatternError::InvalidSyntax);
                }
                out.push('|');
            }
            b'*' | b'+' | b'?' => {
                let frame = frames.last_mut().unwrap();
                let start = frame.atom.ok_or(PatternError::InvalidSyntax)?;
                frame.atom_depth += 1;
                frame.depth = frame.depth.max(frame.atom_depth);
                if frame.depth > depth_limit {
                    return Err(PatternError::Resource(ResourceLimit::GroupDepth));
                }
                // Each quantifier applies to the preceding expression, never as laziness.
                out.insert_str(start, "(?:");
                out.push(')');
                out.push(char::from(byte));
            }
            _ => {
                frames.last_mut().unwrap().atom = Some(out.len());
                frames.last_mut().unwrap().atom_depth = 0;
                match byte {
                    b'^' | b'$' => emit_byte(&mut out, b'\n'),
                    b'.' => out.push_str("[^\\x0A]"),
                    b'[' => emit_set(&mut out, &bracket(input, &mut p)?),
                    b'\\' => {
                        let escaped = *input.get(p).ok_or(PatternError::InvalidSyntax)?;
                        p += 1;
                        emit_byte(&mut out, escaped);
                    }
                    literal => emit_byte(&mut out, literal),
                }
            }
        }
    }
    if frames.len() != 1 || frames[0].atom.is_none() {
        return Err(PatternError::InvalidSyntax);
    }
    Ok(out)
}

fn bracket(input: &[u8], p: &mut usize) -> Result<ByteSet, PatternError> {
    let negated = input.get(*p) == Some(&b'^');
    *p += usize::from(negated);
    let mut set = [false; 256];
    let mut previous = None;
    loop {
        let byte = *input.get(*p).ok_or(PatternError::InvalidSyntax)?;
        *p += 1;
        if byte == b'-'
            && let Some(start) = previous
        {
            let end = *input.get(*p).ok_or(PatternError::InvalidSyntax)?;
            *p += 1;
            // A range of bytes, unsigned, as the reference reads it in the C locale.
            if start <= end {
                set[usize::from(start)..=usize::from(end)].fill(true);
                previous = Some(end);
            }
        } else {
            set[usize::from(byte)] = true;
            previous = Some(byte);
        }
        if input.get(*p) == Some(&b']') {
            *p += 1;
            if negated {
                set.iter_mut().for_each(|included| *included = !*included);
                // A negated set never matches a line break, even when another branch has
                // anchors.
                set[usize::from(b'\n')] = false;
            }
            return Ok(set);
        }
    }
}
