typedef int API scalar;
typedef Type API aliased;
typedef unsigned API keyword;
typedef T A B several_words;
typedef int API *before_pointer;
typedef int API array[3];
typedef struct Tag { int member; } API aggregate;
typedef struct { int member; } API anonymous;
typedef int API mixed, other;
typedef int API (*function_pointer)(void);
typedef int ordinary;
#define API
typedef int API macro_defined;
void local(void) { typedef int API local_alias; }

/* Pointer-after-star and the ratified parenthesized exceptions stay. */
typedef int * AFTER_STAR pointer;
typedef const QualifiedType * AFTER_STAR qualified_pointer;
typedef CONST sds(alias);
typedef unsigned sds(alias);
typedef int actual __attribute__((unused));
typedef int windows __declspec(align(8));

/* An opaque invocation before the alias is outside the word-prefix policy. */
#define RET_TYPE(T) T
#define ATTR
typedef RET_TYPE(int) invocation_alias ATTR;
invocation_alias invocation_instance;

/* The ratified written preference cannot identify an empty suffix macro. */
typedef int suffix_alias ATTR;
suffix_alias suffix_instance;

typedef int
#define NOISE * (unclosed
API after_macro;
typedef int
#ifdef FIRST
*
#else
API in_other_arm,
#endif
AFTER_JOIN joined;
typedef int
#ifdef FIRST
RET_TYPE(int)
#else
API in_word_arm,
#endif
AFTER_GROUP group_joined;

/* Unsupported or incomplete groups and nonword prefixes stay unchanged. */
typedef RET_TYPE(int) before_group later_name;
typedef int * AFTER_STAR2 other_pointer;
typedef int number_prefix 1 API numeric_tail;
typedef int string_prefix "text" API string_tail;
typedef int operator_prefix + API operator_tail;
typedef int API incomplete(unknown);
typedef int API nested((alias))(void);
typedef int API unfinished[
