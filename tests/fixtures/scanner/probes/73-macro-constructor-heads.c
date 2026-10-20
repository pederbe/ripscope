int helper(void) { return 1; }
#define CppConcat(left, right) left ## right
#define GENERATED(stat) \
    int \
    CppConcat(generated_, stat)(void) \
    { return helper(); }
GENERATED(value)
#undef CppConcat
int CppConcat(int value) { return value; }
int ordinary(void) { return CppConcat(helper()); }
#define CTOR_SAME(stat) static int CppConcat(same_, stat)(void) { return helper(); }
typedef int Datum;
#define PG_FUNCTION_ARGS void
#define GENERATED_OPAQUE(stat) \
    Datum \
    CppConcat(generated_, stat)(PG_FUNCTION_ARGS) \
    { return helper(); }
#define TRIVIA(stat) /* prefix */ static /* gap */ Datum \
    CppConcat(generated_ /* left */, /* right */ stat) \
    /* parameters */ (PG_FUNCTION_ARGS) /* body */ { return helper(); }
#define OBJECT_HEAD int CppConcat(object_, name)(void) { helper(); }
#define DIRECT(NAME) static int NAME(void) { return helper(); }
#define SPLIT(NAME) static int \
    NAME(void) { return helper(); }
#define INVALID(stat) int; Datum CppConcat(body_, stat)(void) { helper(); }
#define BODY(stat) do { CppConcat(body_, stat)(value); helper(); } while (0)
#define SPLICED_HEADER\
(stat) int Ctor(left, right)(PG_FUNCTION_ARGS) { return helper(); }
