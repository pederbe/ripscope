#ifdef __cplusplus
extern "C" {
#endif
extern int ext_decl(int x);
API_DATA(long) ref_total;
API_DATA(const struct frozen *) frozen_modules;
API_DATA(int) digit_values[];
EXPORT_C __declspec(selectany) ulong_t enable_bits[1];
API_FUNC(int) api_function(int x);
char *attributed(int n) FAST_ATTR;
implicit(int x) ATTR2;
M(T) name_attr ATTR4;
M(x) struct tagfwd;
A(x) B(y) twomac;
#ifdef __cplusplus
}
#endif
static __declspec(thread) struct state tls_state;
__declspec(selectany) handle_t *global_handle;
API_DATA(int) outside_block;
int last;
