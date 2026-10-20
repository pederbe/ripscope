int helper(void) { return 1; }
#define GENERATED_FUNCTION(NAME) \
    static int NAME(void) { return helper(); }
#define INVOKE_PARAMETER(NAME) NAME()
int ordinary(void) { return helper(); }
int prototype(void);
#define LONG_HEAD(NAME) \
    static void NAME(const unsigned char* restrict first,                    \
                     const unsigned char* restrict second,                   \
                     const unsigned char* restrict third,                    \
                     unsigned char* restrict output,                         \
                     int length) { helper(); }
#define COMMENT_HEAD(NAME) static int NAME /* parameters */ (void) /* body */ { return helper(); }
#define SPLICE_HEAD(NAME) static int NAME \
    (void) \
    { return helper(); }
#define CALLBACK_HEAD(NAME) \
    static int NAME(int (*callback)(void), int value) { return callback() + helper(); }
#define GENERATED_PROTOTYPE(NAME) static int NAME(void);
#define INVOKE_BLOCK(NAME) NAME(1) { helper(); }
#define LOCAL_CALLS(NAME) do { int value = NAME(); helper(); } while (0)
#define ANNOTATED_HEAD(NAME) static int NAME(void) SUFFIX { return helper(); }
#define SPLIT_PREFIX(NAME) static int \
    NAME(void) { return helper(); }
#define UNFINISHED_HEAD(NAME) static int NAME(void)
{ helper(); }
int after_macros(void) { return helper(); }
