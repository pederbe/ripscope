#define API(type) type
int helper(void) { return 1; }
API(int) wrapped(void) { return helper(); }
#undef API
int API(int value) { return value; }
int call_api(void) { return API(3); }
#define RETTYPE(type) type
RETTYPE(const char *) pointer_wrapped(void) { return 0; }
RETTYPE(Node *) opaque_pointer_wrapped(int value) { return helper(); }
RETTYPE(Node) opaque_wrapped(void) { return helper(); }
RETTYPE(int) prototype(void);
int suffix_real(int value) RETTYPE(value) { return helper(); }
RETTYPE(1) numeric_annotation(void) { return helper(); }
RETTYPE(int) nested_return(void (*cb)(int)) { cb(1); }
RETTYPE(int) callback_head(int callback(int)) { return callback(1); }
#define WRAP(x) WRAP_##x
#define WRAP_int
#define ATTR
int WRAP(int retained_inner(int value)) ATTR { return value; }
RETTYPE(int) ANNOTATED decorated(void) { return helper(); }
RETTYPE(int) annotated(void) ATTR { return helper(); }
RETTYPE((int)) nested_operand(void) { return helper(); }
struct Container { RETTYPE(int) (*member)(int value); };
int after;
#define EMPTY(arg)
implicit_api(void) EMPTY(void) { return 1; }
int implicit_call(void) { return implicit_api(); }
#define GNU_EMPTY(arg)
gnu_real(int) GNU_EMPTY(void) { return 1; }
int gnu_call(void) { return gnu_real(2); }
