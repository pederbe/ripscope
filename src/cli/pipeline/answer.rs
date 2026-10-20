//! Queries and their answers: a query compiled for the transport, and the rows the query
//! engine or a live text search finds, written for the transport and spooled so the
//! count and any warnings are known before a row reaches the reader.

use crate::engine::{self, Field, Reads, Semantics};
use crate::invocation::Invocation;
use crate::query::{IdentifierCase, Query, Refusal, StoredQuery, TextQuery};
use crate::refresh::path_of;
use crate::revision::{self, Index, LeftOut, Revision, Units};
use crate::sink::{self, Answer, Sink, Transport};
use crate::store::{REBUILD, StoreError};
use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;

/// The query semantics the product answers with (ADR 0010).
const SEMANTICS: Semantics = Semantics::Own;

/// Why a query cannot be answered at all, as a transport tells it.
pub(super) struct Unusable {
    /// Short text, which Vim's error prompt can show: one line that holds no `>`.
    pub message: Cow<'static, str>,
    /// What went wrong in detail, where the transport can carry any text.
    pub detail: Option<String>,
}

impl Unusable {
    pub(super) fn new(message: &'static str) -> Self {
        Self {
            message: Cow::Borrowed(message),
            detail: None,
        }
    }

    /// The reason in full.
    pub(super) fn reason(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.detail.as_deref().unwrap_or(&self.message))
    }
}

/// Compiles `pattern` for query field `field` (0 to 9), for `transport`.
pub(super) fn compile(
    field: u8,
    pattern: &[u8],
    case: IdentifierCase,
    transport: Transport,
) -> Result<Query<'_>, Unusable> {
    let query = Query::compile(field, pattern, case).map_err(|refusal| match refusal {
        Refusal::Nul => Unusable::new("the query pattern holds a NUL byte"),
        Refusal::NoField => Unusable::new("no such query field"),
        Refusal::Pattern(error) => Unusable {
            message: Cow::Borrowed("the query pattern cannot be used"),
            detail: Some(format!("invalid query pattern: {error}")),
        },
    })?;
    // Field 1 shows the pattern, its trailing blanks trimmed as cscope trims them, as each
    // row's context column.
    if let Query::Stored(stored) = &query
        && stored.field() == Field::Definitions
        && transport == Transport::Line
        && !sink::fits_column(stored.shown_pattern())
    {
        return Err(Unusable {
            message: Cow::Borrowed("the definition pattern cannot be shown in a row"),
            detail: Some(
                "a definition pattern with blanks cannot be shown in the line protocol; use \
                 --json"
                    .into(),
            ),
        });
    }
    Ok(query)
}

/// Why a query found nothing to deliver.
pub(super) enum Failed {
    /// The index cannot be read, or holds nothing published.
    Index(String),
    /// Writing the answer failed.
    Output(io::Error),
}

impl From<io::Error> for Failed {
    fn from(error: io::Error) -> Self {
        Self::Output(error)
    }
}

/// Answers `query` from the index's current revision, read for what the query needs, for
/// the invocation's transport and `-P` path; `units` keeps what a session decoded.
pub(super) fn find(
    query: &Query<'_>,
    invocation: &Invocation,
    index: Index<'_>,
    units: &mut Units,
) -> Result<Answer, Failed> {
    find_in_parts(query, invocation, index, units, revision::PART, walkers)
}

/// The most threads that walk a stored query's parts while this one reads them. On the
/// Linux kernel, with four cores, three take a common name's query to about half its time.
const MOST_WALKERS: usize = 3;

/// How many threads walk a stored query's parts while this one reads them: the cores but
/// one, and at most [`MOST_WALKERS`].
fn walkers() -> usize {
    static WALKERS: OnceLock<usize> = OnceLock::new();
    *WALKERS.get_or_init(|| {
        thread::available_parallelism()
            .map_or(1, |cores| cores.get())
            .saturating_sub(1)
            .min(MOST_WALKERS)
    })
}

/// [`find`], reading the revision in parts of `part_bytes` bytes of unit images, walked
/// on as many other threads as `walkers` gives (none: on this one).
fn find_in_parts(
    query: &Query<'_>,
    invocation: &Invocation,
    index: Index<'_>,
    units: &mut Units,
    part_bytes: usize,
    walkers: impl FnOnce() -> usize,
) -> Result<Answer, Failed> {
    let mut sink = Sink::new(invocation.transport);
    let warnings = match query {
        Query::Stored(query) => stored(query, index, units, part_bytes, walkers, &mut sink)?,
        Query::Text(query) => {
            let revision = Revision::read(index, &Reads::Paths, units).map_err(Failed::Index)?;
            text(&revision, query, invocation.prefix.as_deref(), &mut sink)?
        }
    };
    Ok(sink.finish(warnings)?)
}

/// One part's answer, written for the transport and spooled: its rows at file scope, its
/// other rows, and what it left out.
struct Answered {
    sink: Sink,
    left_out: LeftOut,
    postings_damaged: bool,
}

/// A part's number, its answer from a walking thread or the walk's panic, and the part,
/// which goes back to the thread that read it to be let go: freeing units on the thread
/// that decoded them was faster (on the Linux kernel, `-0 NULL` took 13% less).
type Walked = (usize, thread::Result<io::Result<Answered>>, Revision);

/// Answers `query` from one part into the result sink.
fn answer_into(query: &StoredQuery<'_>, part: &Revision, sink: &mut Sink) -> io::Result<()> {
    profile_span!("walk_and_spool");
    // Tests make the walk of a file of one of these names panic, or wait.
    #[cfg(test)]
    for path in part.paths() {
        assert!(path != b"panics.c", "a walk failed");
        if path == b"slow.c" {
            thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    for row in query.answer(SEMANTICS, part) {
        sink.row(row)?;
    }
    Ok(())
}

/// Answers `query` from one part on a walking thread, its rows spooled for `transport`.
fn answer_part(
    query: &StoredQuery<'_>,
    part: &Revision,
    transport: Transport,
) -> io::Result<Answered> {
    let mut sink = Sink::part(transport);
    answer_into(query, part, &mut sink)?;
    Ok(Answered {
        sink,
        left_out: part.left_out(),
        postings_damaged: part.postings_damaged(),
    })
}

/// The parts' answers as they are written to the answer's spools, in the parts' order
/// whatever order they come in.
struct Written<'s> {
    sink: &'s mut Sink,
    left_out: LeftOut,
    postings_damaged: bool,
    next: usize,
    waiting: BTreeMap<usize, Answered>,
}

impl Written<'_> {
    /// Takes the answer of the part numbered `number`, and writes those whose turn has come.
    fn take(&mut self, number: usize, answered: Answered) -> io::Result<()> {
        debug_assert!(number >= self.next && !self.waiting.contains_key(&number));
        self.waiting.insert(number, answered);
        while let Some(answered) = self.waiting.remove(&self.next) {
            self.left_out += answered.left_out;
            self.postings_damaged |= answered.postings_damaged;
            self.sink.absorb(answered.sink)?;
            self.next += 1;
        }
        Ok(())
    }

    /// Answers `query` here from the part numbered `number`, whose turn it is: its rows go
    /// straight to the answer's spools.
    fn walk_here(
        &mut self,
        query: &StoredQuery<'_>,
        number: usize,
        part: &Revision,
    ) -> io::Result<()> {
        debug_assert!(number == self.next && self.waiting.is_empty());
        self.left_out += part.left_out();
        self.postings_damaged |= part.postings_damaged();
        answer_into(query, part, self.sink)?;
        self.next += 1;
        Ok(())
    }

    /// Takes an answer a walking thread gave, or carries on its walk's panic here.
    fn take_walked(&mut self, (number, walked, part): Walked) -> io::Result<()> {
        drop(part);
        match walked {
            Ok(answered) => self.take(number, answered?),
            Err(panic) => panic::resume_unwind(panic),
        }
    }
}

/// The parts out at once, handed to the walking threads and not yet written: one for each
/// walking thread whatever its size, and more while they hold at most so many bytes of
/// unit images, since a slow part holds back the answers after it.
struct Out {
    /// Each part's number and bytes of images, oldest first.
    parts: VecDeque<(usize, usize)>,
    /// How many threads walk them.
    walkers: usize,
    /// The most bytes of images they hold once there are more parts than walking threads.
    most: usize,
}

impl Out {
    fn new(walkers: usize, most: usize) -> Self {
        Self {
            parts: VecDeque::new(),
            walkers,
            most,
        }
    }

    /// Whether a part of `bytes` bytes may go out, once the parts numbered below
    /// `unwritten` are written.
    fn admits(&mut self, unwritten: usize, bytes: usize) -> bool {
        while self
            .parts
            .front()
            .is_some_and(|&(number, _)| number < unwritten)
        {
            self.parts.pop_front();
        }
        let held: usize = self.parts.iter().map(|&(_, bytes)| bytes).sum();
        self.parts.len() < self.walkers || held + bytes <= self.most
    }

    fn add(&mut self, number: usize, bytes: usize) {
        self.parts.push_back((number, bytes));
    }
}

#[cfg(test)]
thread_local! {
    /// The most parts a query read on this thread had out at once.
    static MOST_OUT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A stored field, answered by the query engine from the revision read in parts: rows at
/// file scope come first, with their ordering and spooling owned by `sink`, so that no
/// part's units outlive it. This thread reads the parts and, once there is a second part, as
/// many other threads as `walkers` gives walk them while it reads the next, each part's
/// rows spooled until its turn comes. Returns the warnings for the files it cannot
/// read.
fn stored(
    query: &StoredQuery<'_>,
    index: Index<'_>,
    units: &mut Units,
    part_bytes: usize,
    walkers: impl FnOnce() -> usize,
    sink: &mut Sink,
) -> Result<Vec<String>, Failed> {
    let reads = query.reads(SEMANTICS);
    let transport = sink.transport();
    let mut written = Written {
        sink,
        left_out: LeftOut::default(),
        postings_damaged: false,
        next: 0,
        waiting: BTreeMap::new(),
    };
    let (parts, jobs) = mpsc::channel::<(usize, Revision)>();
    let jobs = Mutex::new(jobs);
    let (send, answers) = mpsc::channel::<Walked>();
    thread::scope(|scope| -> Result<(), Failed> {
        // The first part waits for a second: a query that reads one is walked here, without
        // asking how many cores there are.
        let mut first = None;
        let mut walkers = Some(walkers);
        // The parts out, once other threads walk them.
        let mut walking: Option<Out> = None;
        let mut next = 0;
        let read = Revision::read_in_parts(index, &reads, units, part_bytes, |part| {
            let number = next;
            next += 1;
            if number == 0 {
                first = Some(part);
                return Ok(());
            }
            if let Some(walkers) = walkers.take() {
                let mut started = 0;
                for _ in 0..walkers() {
                    let (jobs, send) = (&jobs, send.clone());
                    let walker = thread::Builder::new().name("walk".into());
                    let walker = walker.spawn_scoped(scope, move || {
                        // Until the reading ends. A walk that panics hands its panic to the
                        // reading thread, which would otherwise wait for its answer.
                        while let Some((number, part)) =
                            jobs.lock().ok().and_then(|jobs| jobs.recv().ok())
                        {
                            let walked = panic::catch_unwind(AssertUnwindSafe(|| {
                                answer_part(query, &part, transport)
                            }));
                            let panicked = walked.is_err();
                            if send.send((number, walked, part)).is_err() || panicked {
                                break;
                            }
                        }
                        #[cfg(feature = "session-profile")]
                        crate::profile::flush();
                    });
                    // The system may refuse a thread: those that started walk the parts,
                    // or, with none, this one.
                    if walker.is_err() {
                        break;
                    }
                    started += 1;
                }
                let held = first.take().expect("the first part");
                if started == 0 {
                    written.walk_here(query, 0, &held)?;
                } else {
                    // Two parts' worth of images for each walking thread.
                    let mut out = Out::new(started, part_bytes.saturating_mul(2 * started));
                    out.add(0, held.image_bytes());
                    parts
                        .send((0, held))
                        .expect("the walkers' queue outlives the reading");
                    walking = Some(out);
                }
            }
            let Some(out) = &mut walking else {
                return written.walk_here(query, number, &part);
            };
            let bytes = part.image_bytes();
            while !out.admits(written.next, bytes) {
                // Every part out is answered, or its walk's panic comes instead.
                let walked = answers.recv().expect("this thread holds a sender");
                written.take_walked(walked)?;
            }
            out.add(number, bytes);
            #[cfg(test)]
            MOST_OUT.with(|most| most.set(most.get().max(number + 1 - written.next)));
            parts
                .send((number, part))
                .expect("the walkers' queue outlives the reading");
            while let Ok(walked) = answers.try_recv() {
                written.take_walked(walked)?;
            }
            Ok(())
        });
        drop((parts, send));
        if let Some(part) = first {
            written.walk_here(query, 0, &part)?;
        }
        for walked in answers {
            written.take_walked(walked)?;
        }
        Ok(read.map_err(Failed::Index)??)
    })?;
    debug_assert!(written.waiting.is_empty());
    let Written {
        left_out,
        postings_damaged,
        ..
    } = written;
    let mut warnings = Vec::new();
    if left_out.other_version > 0 {
        warnings.push(revision::other_version(left_out.other_version));
    }
    if postings_damaged {
        warnings.push(StoreError::Postings.to_string());
    }
    if left_out.damaged > 0 {
        let (files, are) = if left_out.damaged == 1 {
            ("file", "is")
        } else {
            ("files", "are")
        };
        warnings.push(format!(
            "damaged index: {} indexed {files} cannot be read and {are} left out of the \
             answers; {REBUILD}",
            left_out.damaged
        ));
    }
    Ok(warnings)
}

/// Fields 4, 5 and 6 read the indexed files as they are now; a file that cannot be read
/// is skipped with a warning, which it returns. Lines are matched and shown as cscope
/// does: with a CR LF ending's carriage return. Under `-P`, a relative name is read and
/// shown below its path, as cscope's text searches do (`find.c`, `filepath()`). Files are
/// read and searched on several threads, and answered in order.
fn text(
    revision: &Revision,
    query: &TextQuery<'_>,
    prefix: Option<&[u8]>,
    sink: &mut Sink,
) -> io::Result<Vec<String>> {
    let files: Vec<(Cow<'_, [u8]>, PathBuf)> = revision
        .paths()
        .map(|path| {
            let name = path_of(path);
            match prefix {
                Some(prefix) if !name.is_absolute() => {
                    let mut shown = prefix.to_vec();
                    shown.push(b'/');
                    shown.extend_from_slice(path);
                    let full = path_of(&shown);
                    (Cow::Owned(shown), full)
                }
                _ => (Cow::Borrowed(path), revision.root().join(name)),
            }
        })
        .collect();
    let searched = search_files(&files, query);
    let mut warnings = Vec::new();
    for ((shown, _), found) in files.iter().zip(searched) {
        match found {
            Ok(lines) => {
                for (number, line) in lines {
                    sink.text_row(shown, engine::UNKNOWN, number, &line)?;
                }
            }
            Err(error) => warnings.push(format!(
                "cannot read {}: {error}",
                String::from_utf8_lossy(shown)
            )),
        }
    }
    Ok(warnings)
}

/// The most threads that read and search files for a text search.
const MOST_SEARCHERS: usize = 8;

/// Each file's matching lines, with their numbers and without their line feed, or why it
/// cannot be read, in the files' order.
type Searched = io::Result<Vec<(u32, Vec<u8>)>>;

/// Reads and searches `files` on as many threads as there are cores, up to
/// [`MOST_SEARCHERS`], each taking the next file as it finishes one.
fn search_files(files: &[(Cow<'_, [u8]>, PathBuf)], query: &TextQuery<'_>) -> Vec<Searched> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let search = || {
        let (mut bytes, mut scratch, mut folded) = (Vec::new(), Vec::new(), Vec::new());
        let mut found = Vec::new();
        loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some((_, full)) = files.get(at) else {
                return found;
            };
            found.push((
                at,
                search_file(full, query, &mut bytes, &mut scratch, &mut folded),
            ));
        }
    };
    let threads = thread::available_parallelism()
        .map_or(1, |cores| cores.get())
        .min(MOST_SEARCHERS)
        .min(files.len());
    let mut found: Vec<(usize, Searched)> = if threads <= 1 {
        search()
    } else {
        thread::scope(|scope| {
            let others: Vec<_> = (1..threads)
                .filter_map(|_| thread::Builder::new().spawn_scoped(scope, search).ok())
                .collect();
            let mut found = search();
            for other in others {
                found.extend(
                    other
                        .join()
                        .unwrap_or_else(|panic| panic::resume_unwind(panic)),
                );
            }
            found
        })
    };
    found.sort_unstable_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, searched)| searched).collect()
}

/// One file's matching lines, read into `bytes`.
fn search_file(
    full: &Path,
    query: &TextQuery<'_>,
    bytes: &mut Vec<u8>,
    scratch: &mut Vec<u8>,
    folded: &mut Vec<u8>,
) -> Searched {
    use std::io::Read;
    bytes.clear();
    std::fs::File::open(full)?.read_to_end(bytes)?;
    let mut lines = Vec::new();
    let Ok(()) = query.matching_lines(bytes, scratch, folded, |number, line| {
        let number = u32::try_from(number).unwrap_or(u32::MAX);
        lines.push((number, line.strip_suffix(b"\n").unwrap_or(line).to_vec()));
        Ok::<(), Infallible>(())
    });
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation;
    use crate::revision::tests::{build, build_in_blocks};
    use crate::sink::WarningStyle;
    use std::path::Path;

    fn body(found: &mut Answer) -> Vec<u8> {
        let mut out = Vec::new();
        found
            .deliver(&mut out, &mut io::sink(), WarningStyle::Stderr, |_, _| {
                Ok(())
            })
            .unwrap();
        out
    }

    /// The answer to the one-shot query `args` from `index`, read in parts of `part_bytes`
    /// walked on as many other threads as `walkers` gives: its row count, warnings and
    /// output, and the most parts it had out at once. It is found on another thread within a
    /// minute, so that a query that would wait for ever fails the test instead.
    fn answer_and_out(
        index: &Path,
        args: &[&str],
        part_bytes: usize,
        walkers: impl FnOnce() -> usize + Send + 'static,
    ) -> ((usize, Vec<String>, String), usize) {
        let mut args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
        args.extend(["-f".into(), index.into()]);
        let (send, done) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(panic::catch_unwind(AssertUnwindSafe(|| {
                let invocation = invocation::parse(&args).unwrap();
                let invocation::Action::Query { field, pattern } = &invocation.action else {
                    panic!("a query");
                };
                let case = IdentifierCase::Sensitive;
                let query = compile(*field, pattern, case, invocation.transport)
                    .ok()
                    .unwrap();
                let units = &mut Units::none();
                MOST_OUT.with(|most| most.set(0));
                let index = (&invocation.index).into();
                let found = find_in_parts(&query, &invocation, index, units, part_bytes, walkers);
                let Ok(mut found) = found else {
                    panic!("answered");
                };
                let out = body(&mut found);
                let answer = (
                    found.rows(),
                    found.warnings().to_vec(),
                    String::from_utf8(out).unwrap(),
                );
                (answer, MOST_OUT.with(std::cell::Cell::get))
            })));
        });
        let found = done.recv_timeout(std::time::Duration::from_secs(60));
        found
            .expect("answered in time")
            .unwrap_or_else(|panic| panic::resume_unwind(panic))
    }

    /// [`answer_and_out`]'s answer.
    fn answer(
        index: &Path,
        args: &[&str],
        part_bytes: usize,
        walkers: impl FnOnce() -> usize + Send + 'static,
    ) -> (usize, Vec<String>, String) {
        answer_and_out(index, args, part_bytes, walkers).0
    }

    /// Read in parts, of one decoded unit or of a few files, and walked on this thread or on
    /// others, a stored query answers as read whole: every part's rows at file scope, then
    /// every part's other rows, with the warnings of every part.
    #[test]
    fn an_answer_read_in_parts_is_the_answer_read_whole() {
        let work = tempfile::tempdir().unwrap();
        let file = "#include \"a.h\"\nint Alpha;\nvoid f(void) { Alpha = g(Alpha); }\n";
        let index = build(
            work.path(),
            &[
                ("a.c", file),
                ("b.c", "int Beta;\n"),
                ("c.c", &file.replace("f(void)", "h(void)")),
                ("d.c", "int Delta;\n"),
                ("e.c", file),
            ],
            &["a.c", "b.c", "c.c", "d.c", "e.c"],
        );
        // b.c's unit and d.c's are damaged, so that they fall in different parts.
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute(
                "UPDATE blocks SET image = x'00' WHERE id >> 20 IN (SELECT unit FROM files \
                 WHERE path IN (CAST('b.c' AS BLOB), CAST('d.c' AS BLOB)))",
                [],
            )
            .unwrap();
        let ask = |args: &[&str], part_bytes| answer(&index, args, part_bytes, || 1);
        for query in [
            ["-dL", "-0Alpha"],
            ["-dL", "-0.*"],
            ["-dL", "-1Alpha"],
            ["-dL", "-2f"],
            ["-dL", "-3g"],
            ["-dL", "-7.*"],
            ["-dL", "-8a.h"],
            ["-dL", "-9Alpha"],
            ["-dL", "-0Nothing"],
            ["--json", "-dL0Alpha"],
        ] {
            // Read whole, in one part, it does not ask how many threads may walk it.
            let whole = answer(&index, &query, usize::MAX, || unreachable!());
            for walkers in [0, 1, 3] {
                let parts = answer(&index, &query, 1, move || walkers);
                assert_eq!(parts, whole, "{query:?} {walkers}");
            }
            // Parts of a.c to c.c and of d.c and e.c, each with a damaged unit, and the
            // unit a.c and e.c share in both.
            assert_eq!(ask(&query, 150), whole, "{query:?}");
        }
        let (rows, warnings, out) = ask(&["-dL", "-0Alpha"], 1);
        assert_eq!((rows, warnings), (6, Vec::<String>::new()));
        assert!(out.starts_with("a.c <global> 2 int Alpha;\nc.c <global> 2 int Alpha;\n"));
        assert!(out.ends_with("e.c f 3 void f(void ) { Alpha = g(Alpha); }\n"));
        // `.*` matches a name in every file, so it reads every file, and each damaged one
        // is counted.
        let (_, warnings, _) = ask(&["-dL", "-0.*"], 1);
        assert_eq!(
            warnings,
            [
                "damaged index: 2 indexed files cannot be read and are left out of the answers; \
                 rebuild it with -bu and the options it was built with"
            ]
        );
        // A name whose postings cannot be read is looked for in every file: the answer is
        // whole, and warns of the damage it met.
        let (rows, _, out) = ask(&["-dL", "-0Alpha"], 1);
        connection
            .execute(
                "UPDATE names SET postings = x'80' WHERE name = CAST('Alpha' AS BLOB)",
                [],
            )
            .unwrap();
        let (again, warnings, same) = ask(&["-dL", "-0Alpha"], 1);
        assert_eq!((again, same), (rows, out));
        assert_eq!(
            warnings,
            [
                "damaged index: its name postings cannot be read; rebuild it with -bu and the \
                 options it was built with",
                "damaged index: 2 indexed files cannot be read and are left out of the answers; \
                 rebuild it with -bu and the options it was built with",
            ]
        );
    }

    /// A query's row count, warnings and output.
    type Reply = (usize, Vec<String>, Vec<u8>);

    /// The answers to `queries` (each its arguments without `-f`) from `index`, as
    /// [`answer`] gives them, found in turn with one cache that keeps `budget` bytes, as a
    /// session finds them; a pattern a query cannot use gives no answer. They are found on
    /// another thread within five minutes.
    fn answers(
        index: &Path,
        queries: &[Vec<String>],
        part_bytes: usize,
        walkers: usize,
        budget: usize,
    ) -> Vec<Option<Reply>> {
        let (index, queries) = (index.to_owned(), queries.to_vec());
        let (send, done) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(panic::catch_unwind(AssertUnwindSafe(|| {
                let mut units = Units::with_budget(budget);
                let mut answers = Vec::new();
                for query in &queries {
                    let mut args: Vec<std::ffi::OsString> = query.iter().map(Into::into).collect();
                    args.extend(["-f".into(), index.clone().into()]);
                    let invocation = invocation::parse(&args).unwrap();
                    let invocation::Action::Query { field, pattern } = &invocation.action else {
                        panic!("a query");
                    };
                    let case = super::super::case(&invocation);
                    let Ok(query) = compile(*field, pattern, case, invocation.transport) else {
                        answers.push(None);
                        continue;
                    };
                    let found = find_in_parts(
                        &query,
                        &invocation,
                        (&invocation.index).into(),
                        &mut units,
                        part_bytes,
                        || walkers,
                    );
                    let Ok(mut found) = found else {
                        panic!("answered");
                    };
                    let out = body(&mut found);
                    answers.push(Some((found.rows(), found.warnings().to_vec(), out)));
                }
                answers
            })));
        });
        let found = done.recv_timeout(std::time::Duration::from_secs(300));
        found
            .expect("answered in time")
            .unwrap_or_else(|panic| panic::resume_unwind(panic))
    }

    /// Over an index of a block for each line (ADR 0011), every query of the reference
    /// probes, and each name their files hold in fields 0 to 3 and 9, answers as over an
    /// index of whole units: read whole, in parts of a byte walked here or on three other
    /// threads, and in a session whose cache keeps a few blocks or all of them.
    #[test]
    fn blocks_of_a_line_answer_as_whole_units() {
        let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/queries");
        let mut probes: Vec<PathBuf> = std::fs::read_dir(probes)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .collect();
        probes.sort();
        let mut asked = 0;
        for probe in probes {
            let expected: serde_json::Value =
                serde_json::from_slice(&std::fs::read(probe.join("expected.json")).unwrap())
                    .unwrap();
            let files: Vec<&str> = expected["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap())
                .collect();
            let mut queries: Vec<Vec<String>> = Vec::new();
            for query in expected["queries"].as_array().unwrap() {
                let mut args = vec!["-dL".to_owned()];
                if query["caseless"].as_bool().unwrap() {
                    args.push("-C".into());
                }
                let (field, pattern) = (&query["field"], &query["pattern"]);
                args.push(format!(
                    "-{}{}",
                    field.as_str().unwrap(),
                    pattern.as_str().unwrap()
                ));
                queries.push(args);
            }
            let (whole, blocks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let mut names = std::collections::BTreeSet::new();
            for name in &files {
                let source = std::fs::read(probe.join(name)).unwrap();
                for work in [&whole, &blocks] {
                    std::fs::write(work.path().join(name), &source).unwrap();
                }
                let words = source.split(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'));
                names.extend(
                    words
                        .filter(|word| word.first().is_some_and(|b| !b.is_ascii_digit()))
                        .map(|word| String::from_utf8_lossy(word).into_owned()),
                );
            }
            for name in names {
                for field in "01239".chars() {
                    queries.push(vec!["-dL".into(), format!("-{field}{name}")]);
                }
            }
            let whole = build(whole.path(), &[], &files);
            let blocks = build_in_blocks(blocks.path(), &[], &files, Some(1));
            let (stored, units): (i64, i64) = rusqlite::Connection::open(&blocks)
                .unwrap()
                .query_row(
                    "SELECT (SELECT count(*) FROM blocks), (SELECT count(*) FROM units)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert!(stored > 3 * units, "{stored} blocks of {units} units");
            let want = answers(&whole, &queries, usize::MAX, 0, 0);
            for (part_bytes, walkers, budget) in [
                (usize::MAX, 0, 0),
                (1, 0, 0),
                (1, 3, 0),
                (usize::MAX, 0, 64),
                (1, 3, 1 << 20),
            ] {
                let got = answers(&blocks, &queries, part_bytes, walkers, budget);
                for ((query, want), got) in queries.iter().zip(&want).zip(&got) {
                    assert_eq!(
                        got,
                        want,
                        "{}: {query:?} {part_bytes} {walkers} {budget}",
                        probe.display()
                    );
                }
            }
            asked += queries.len();
        }
        assert!(asked > 700, "{asked}");
    }

    /// An index of 24 files that each define `Alpha` and call `g`, `bad` among them.
    fn many_files(work: &Path, bad: &str) -> PathBuf {
        let sources: Vec<(String, String)> = (0..24)
            .map(|n| {
                // Read in path order, `bad` comes in the middle.
                let name = match n {
                    0..12 => format!("a{n:02}.c"),
                    12 => bad.to_owned(),
                    _ => format!("z{n:02}.c"),
                };
                let text = format!("int Alpha;\nvoid f{n}(void) {{ Alpha = g(Alpha); }}\n");
                (name, text)
            })
            .collect();
        let sources: Vec<(&str, &str)> = sources
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        let listed: Vec<&str> = sources.iter().map(|(name, _)| *name).collect();
        build(work, &sources, &listed)
    }

    /// Many parts, walked on one other thread or three, a few out at once or more, answer
    /// as one part does, whatever order their answers come back in.
    #[test]
    fn many_parts_answer_as_one() {
        let work = tempfile::tempdir().unwrap();
        let index = many_files(work.path(), "m12.c");
        for query in [["-dL", "-0Alpha"], ["-dL", "-3g"], ["--json", "-dL0Alpha"]] {
            let whole = answer(&index, &query, usize::MAX, || unreachable!());
            for (part_bytes, walkers) in [(1, 1), (1, 3), (100, 3)] {
                let parts = answer(&index, &query, part_bytes, move || walkers);
                assert_eq!(parts, whole, "{query:?} {part_bytes} {walkers}");
            }
        }
    }

    /// A slow part holds back the answers after it, and with them the parts out at once: one
    /// for each walking thread when each part holds more than their allowance of bytes, and
    /// two when each holds one part's worth. The slow walk lasts 100 ms, far longer than
    /// reading the parts after it takes.
    #[test]
    fn a_slow_part_holds_back_few_parts() {
        let work = tempfile::tempdir().unwrap();
        let index = many_files(work.path(), "slow.c");
        let connection = rusqlite::Connection::open(&index).unwrap();
        let largest: i64 = connection
            .query_row(
                "SELECT max(bytes) FROM (SELECT sum(length(image)) AS bytes FROM blocks \
                 GROUP BY id >> 20)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let largest = usize::try_from(largest).unwrap();
        let query = ["-dL", "-0Alpha"];
        let whole = answer(&index, &query, usize::MAX, || unreachable!());
        for walkers in [1, 3] {
            for (part_bytes, most) in [(1, walkers), (largest, 2 * walkers)] {
                let (parts, out) = answer_and_out(&index, &query, part_bytes, move || walkers);
                assert_eq!(parts, whole, "{walkers} {part_bytes}");
                assert_eq!(
                    out, most,
                    "parts out with {walkers} walking, parts of {part_bytes}"
                );
            }
        }
    }

    /// A walk that panics ends the query with its panic, on this thread or on another,
    /// rather than leave it waiting for the walk's answer.
    #[test]
    fn a_walk_that_panics_ends_the_query() {
        let work = tempfile::tempdir().unwrap();
        let index = many_files(work.path(), "panics.c");
        for walkers in [0, 1, 3] {
            let walked =
                panic::catch_unwind(|| answer(&index, &["-dL", "-0Alpha"], 1, move || walkers));
            let panic = walked.expect_err("a panic");
            let message = panic.downcast_ref::<&str>().copied();
            let message = message.or(panic.downcast_ref::<String>().map(String::as_str));
            assert_eq!(message, Some("a walk failed"), "{walkers}");
        }
    }

    /// A part that holds nothing, as a walking thread hands back.
    fn part() -> Revision {
        Revision::in_memory(Vec::new())
    }

    /// The parts' answers are written in the parts' order whatever order they arrive in,
    /// spooled in memory or spilled to a file, each one's rows at file scope with the
    /// others', and what they leave out added up; an answer a walk could not write fails.
    #[test]
    fn answers_are_written_in_the_parts_order() {
        let answered = |rows: &[(&str, bool)], damaged: usize, spill: usize| {
            let mut answered = Answered {
                sink: Sink::with_capacity(Transport::Line, spill),
                left_out: LeftOut {
                    other_version: 0,
                    damaged,
                },
                postings_damaged: false,
            };
            for (path, at_file_scope) in rows {
                answered
                    .sink
                    .row(crate::query::ResultRow {
                        path: path.as_bytes(),
                        context: b"<global>",
                        line: 1,
                        text: b"x",
                        at_file_scope: *at_file_scope,
                    })
                    .unwrap();
            }
            answered
        };
        let mut sink = Sink::new(Transport::Line);
        let mut written = Written {
            sink: &mut sink,
            left_out: LeftOut::default(),
            postings_damaged: false,
            next: 0,
            waiting: BTreeMap::new(),
        };
        written.take(2, answered(&[("c", true)], 1, 0)).unwrap();
        written
            .take_walked((1, Ok(Ok(answered(&[("b", false)], 0, 0))), part()))
            .unwrap();
        assert_eq!(written.next, 0);
        written
            .take(0, answered(&[("a", true), ("a", false)], 2, 1024))
            .unwrap();
        assert_eq!((written.next, written.left_out.damaged), (3, 3));
        let unwritten = io::Error::other("no space");
        assert!(
            written
                .take_walked((3, Ok(Err(unwritten)), part()))
                .is_err()
        );
        let mut found = sink.finish(Vec::new()).unwrap();
        assert_eq!(found.rows(), 4);
        assert_eq!(
            body(&mut found),
            b"a <global> 1 x\nc <global> 1 x\na <global> 1 x\nb <global> 1 x\n"
        );
    }

    /// A walk's panic goes on in the thread that writes the answers.
    #[test]
    #[should_panic(expected = "a walk failed")]
    fn a_walk_that_panics_panics_where_answers_are_written() {
        let mut sink = Sink::new(Transport::Line);
        let mut written = Written {
            sink: &mut sink,
            left_out: LeftOut::default(),
            postings_damaged: false,
            next: 0,
            waiting: BTreeMap::new(),
        };
        let walked = (0, Err(Box::new("a walk failed") as _), part());
        let _ = written.take_walked(walked);
    }

    /// The parts out at once are one for each walker whatever its size, and more while
    /// they hold at most so many bytes of images; a part written no longer counts.
    #[test]
    fn parts_out_are_one_a_walker_or_hold_so_many_bytes() {
        let mut out = Out::new(2, 10);
        assert!(out.admits(0, 25));
        out.add(0, 25);
        assert!(out.admits(0, 30));
        out.add(1, 30);
        assert!(!out.admits(0, 1));
        assert!(out.admits(2, 4));
        out.add(2, 4);
        assert!(out.admits(2, 6));
        out.add(3, 6);
        assert!(!out.admits(2, 1));
        assert!(out.admits(3, 4));
        out.add(4, 4);
        assert!(!out.admits(3, 1));
        assert!(out.admits(5, 10));
    }

    /// Rows whose path the line protocol cannot carry are left out wherever they fall,
    /// at the file scope or not, and counted in one warning.
    #[cfg(unix)]
    #[test]
    fn rows_skipped_in_any_part_are_counted_once() {
        let work = tempfile::tempdir().unwrap();
        let file = "int Alpha;\nvoid f(void) { Alpha = 1; }\n";
        let index = build(
            work.path(),
            &[("a\tb.c", file), ("c.c", "int Alpha;\n")],
            &["\"a\tb.c\"", "c.c"],
        );
        for (part_bytes, walkers) in [(1, 3), (1, 0), (usize::MAX, 0)] {
            let (rows, warnings, out) =
                answer(&index, &["-dL", "-0Alpha"], part_bytes, move || walkers);
            assert_eq!((rows, out.as_str()), (1, "c.c <global> 1 int Alpha;\n"));
            assert_eq!(
                warnings,
                ["2 rows skipped: their paths hold control characters; use --json"]
            );
            let (rows, warnings, _) =
                answer(&index, &["-dL", "-1Alpha"], part_bytes, move || walkers);
            assert_eq!(rows, 1);
            assert_eq!(
                warnings,
                ["1 row skipped: its path holds control characters; use --json"]
            );
        }
    }
}
