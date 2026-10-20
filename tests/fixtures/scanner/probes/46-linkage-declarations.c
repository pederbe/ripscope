#ifdef __cplusplus
extern "C" {
#endif
PyAPI_DATA(int) first;
#if defined(WITH_THREADS)
PyAPI_DATA(int) after_if;
#endif
static inline int helper(void) { return 0; }
PyAPI_DATA(int) after_body;
DECLARE(T) initialised = {0};
PyAPI_DATA(int) after_init;
#ifdef __cplusplus
}
#endif
int between;
#ifdef __cplusplus
extern "C" {
#endif
extern int inner(int x);
PyAPI_DATA(int) listed, second;
PyAPI_DATA(int) /* note */ commented;
PyAPI_DATA(int)
    wrapped;
static PyAPI_DATA(int) kept_static;
PyAPI_DATA(int) trailing /* note */;
__declspec(selectany) handle_t t5, t6;
#ifdef __cplusplus
}
#endif
int last;
