T (CALLBACK *handler)(int arg);
struct ops {
HRESULT (STDMETHODCALLTYPE __RPC_FAR *query)(IVim *self, REFID id);
T (API * const *table[4])(int value);
T (API *empty)();
};
extern T (WINAPI *external)(void);
int left, right;
__typeof__(left *right) (product);
TYPEOF(left *right) (macro_product);
TYPEOF(left *right) (*pointer_product);
TYPEOF(left *right) (array_product[(1, 2)]);
T (API opaque)(int arg);
T (*AFTER opaque_after)(int arg);
T (*BETWEEN *opaque_between)(int arg);
T (API *opaque_parameter)(T);
T (API *comment_parameter)(/* comma, */ T);
int x = invoke(API *operand)(int_value, other);
void owner(T (CALLBACK *parameter)(int)) {
T (CALLBACK *local)(int arg);
}
#define DECL T (CALLBACK *macro_name)(int arg)
typedef int (CALLBACK *alias)(int value);
int after;
typedef int callback(int, int);
callback *choose(int value);
int
(wrapper)(int value)
{ return choose(left *right)(value, value); }
