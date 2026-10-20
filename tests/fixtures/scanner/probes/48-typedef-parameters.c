typedef int callback(int count, const T *item);
typedef void (*handler)(int code, void (*nested)(int value));
typedef int (*factory(int input))(int output);
typedef int first(int a), second(int b);
typedef void (CALLBACK *decorated)(int arg);
typedef int scalar;
int after;
int *ptr;
typedef __typeof__(*ptr) (alias);
#define TYPEOF(x) __typeof__(x)
typedef TYPEOF(*ptr) (macro_alias);
typedef T (*unknown)(int arg);
typedef int multiline(
int count,
struct tag { int member; } value);
typedef int next;
void run(void) { typedef int local(int arg); }
int last;
typedef int same(int same);
typedef int API (*x)(void);
typedef int API (CALLBACK *decorated_alias)(void);
typedef int API (bare_alias);
typedef int API (/* comment */ *comment_alias)(void);
typedef int API (int_alias);
typedef int API (void_alias);
typedef int API (*const const_alias)(void);
typedef int qualified(const volatile char *path);
