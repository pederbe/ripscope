# Benchmark answer evidence

[Back to search times](benchmarks.md).

These are the source-backed explanations of every row where Ripscope and cscope answer
differently, in both repetitions on both platforms. Each differing answer was checked
against the pinned source. The release candidate returns the same answers, byte for
byte, as the 16 October 2026 comparison these judgments were first made for.
Session-dependent duplicates and wrong locations in cscope's answers are kept as
observed. Source revisions and query identities are in the
[measurement record](../../evidence/performance-20261019.json).

<details open id="zlib-assigned-control"><summary>zlib · Array initializer</summary><div><p>deflate.c:54 initializes deflate_copyright. Ripscope returns that written assignment. Both cscope configurations return no assignment.</p></div></details>

<details open id="linux-mutation-control"><summary>Sparse Linux kernel · Named mutation</summary><div><p>kernel/fork.c:2454 contains total_forks++. Ripscope finds this written update. Both cscope configurations return no assignment.</p></div></details>

<details open id="redis-caller-control"><summary>Redis · Caller control</summary><div><p>src/dict.h:220 is a declaration of dictSetHashFunctionSeed, not a call. cscope -q reports it as a caller with labels that vary with Session history. Ripscope reports the actual main call at src/server.c:7112.</p></div></details>

<details open id="zlib-callers"><summary>zlib · Callers</summary><div><p>compress.c:53 contains one written deflate call in compress2. In some Sessions plain cscope returns the identical caller row twice. Ripscope retains the three real call rows. Exact variant observations are recorded in the retained answer audit.</p></div></details>

<details open id="zlib-caller-control"><summary>zlib · Caller control</summary><div><p>compress.c:57 has one written deflateEnd call. The observed duplicate plain-cscope row is not an additional source fact. Ripscope retains the four independently labelled calls. This difference occurs only in the variants identified by the retained answer audit.</p></div></details>

<details open id="zlib-includers"><summary>zlib · Includers</summary><div><p>compress.c:9 contains a single include. Some plain-cscope Sessions report that row twice. Ripscope returns the four distinct include rows.</p></div></details>

<details open id="zlib-symbol-regex"><summary>zlib · Symbol pattern</summary><div><p>All configurations return the same 162 locations for deflate.*. Ripscope correctly labels file-scope prototypes and table initializers as global. Plain cscope incorrectly labels 18 rows block_state, a typedef name, while cscope -q mislabels eight. Before Windows timing, a native default/noglob probe confirmed that MSYS=noglob is necessary to pass the literal pattern. All fresh native measurements use that setting.</p></div></details>

<details open id="redis-includers"><summary>Redis · Includers</summary><div><p>src/acl.c:30 contains one include. Some plain-cscope Sessions return it twice, making 61 rows instead of the 60 distinct rows Ripscope returns.</p></div></details>

<details open id="redis-symbol-common"><summary>Redis · Common symbol</summary><div><p>All configurations return the same 3,714 source locations for server. Linux cscope incorrectly labels six rows in emptyDbStructure, emptyData and discardTempDb as global. Native Windows cscope additionally mislabels src/module.c:7681 in moduleBlockClient and :8649 in RM_AddPostNotificationJob. Their function heads and bodies were inspected. Ripscope labels all eight rows correctly.</p></div></details>

<details open id="redis-symbol-regex"><summary>Redis · Symbol pattern</summary><div><p>The same 148 locations are returned for lookupKey.*. Six file-scope prototypes at src/server.h:3232-3237 receive the stale _serverLog context from both cscope configurations. Ripscope correctly labels them global.</p></div></details>

<details open id="linux-assigned"><summary>Sparse Linux kernel · Assignments</summary><div><p>Ripscope retains all 5,676 cscope locations for ret and adds 12 supported writes: eleven increments or decrements and one indexed compound store. It corrects 87 Linux cscope context labels and 146 native Windows cscope labels. All newly differing Windows contexts were checked against the written function heads, body boundaries and result locations in the retained function-context witnesses. Coverage differences remain the same 12 written updates.</p></div></details>

<details open id="linux-callers"><summary>Sparse Linux kernel · Callers</summary><div><p>At kernel/fork.c:2792, alternative SYSCALL_DEFINE6 and SYSCALL_DEFINE5 heads share one written body. Plain cscope retains both heads. Ripscope and cscope -q retain only the last head, a limitation that ADR 0010 describes. Linux plain cscope therefore has an answer-coverage advantage. Native Windows plain cscope also omits the real kernel_thread:2712 and user_mode_thread:2727 calls that Ripscope finds, so the native plain-cscope comparison has advantages on both sides. Native Windows cscope -q returns the six locations but mislabels those two callers as unknown, global, fork_init, bool or kernel_clone according to Session history. Ripscope supplies their actual function names. Such q comparisons have a Ripscope label advantage, while the limitation shared by both tools remains.</p></div></details>

<details open id="linux-includers"><summary>Sparse Linux kernel · Includers</summary><div><p>Ripscope retains the real linux/slab.h includes. Some plain-cscope Sessions substitute a non-include line. The fresh Linux variant substitutes kernel/bpf/ringbuf.c:397, a map_poll initializer, for the real include at :5. The fresh Windows variant substitutes kernel/events/ring_buffer.c:114, a WRITE_ONCE call, for the include at :13. Both sites were inspected directly. Per-variant frequencies remain in the raw audit.</p></div></details>

<details open id="linux-symbol-common"><summary>Sparse Linux kernel · Common symbol</summary><div><p>All configurations return the same 8,871 NULL source locations. Ripscope corrects 106 Linux cscope contexts and 166 native Windows cscope contexts. The additional Windows contexts were checked against source function heads and bodies. At include/linux/wait.h:50 its excerpt also preserves the closing brace present in source and omitted by cscope.</p></div></details>

<details open id="linux-symbol-regex"><summary>Sparse Linux kernel · Symbol pattern</summary><div><p>All configurations return the same 39 kernel_clone.* locations. Both Linux comparators mislabel kernel/vhost_task.c:123 as bool instead of vhost_task_create. Native Windows cscope additionally labels five rows inside create_io_thread, kernel_thread and user_mode_thread as global. Source function heads and body boundaries confirm Ripscope's enclosing function names.</p></div></details>

<details open id="linux-symbol-exact"><summary>Sparse Linux kernel · Symbol</summary><div><p>All configurations return the same eight locations for kernel_clone. Native Windows plain cscope labels the calls at kernel/fork.c:2712 and :2727 global, and cscope -q labels them kernel_clone. These calls are inside kernel_thread and user_mode_thread respectively. Source inspection confirms Ripscope's function labels. The Linux comparators match Ripscope for this request.</p></div></details>

## Pinned sources

- [Linux kernel](https://github.com/torvalds/linux/tree/05f7e89ab9731565d8a62e3b5d1ec206485eeb0b)
- [Redis](https://github.com/redis/redis/tree/ba181057223b04aba580a01b1b36662430aea4a0)
- [zlib](https://github.com/madler/zlib/tree/51b7f2abdade71cd9bb0e7a373ef2610ec6f9daf)

The original row variants, source excerpts and verdicts remain in the
[answer audit](../../evidence/performance-answers-20261016.json).
