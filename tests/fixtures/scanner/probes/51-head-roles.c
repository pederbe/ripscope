int callback_owner(int callback(int)) { return target(); }
int array_owner(int values[table[index()] + bound()]) { return target(); }
static inline void lock(struct lock *p) __acquires(p)
{ target(p); }
void
split_lock(void) __acquires(resource)
{ target(); }
int
#if FLAG
shared_first(void)
#else
shared_second(void)
#endif
{ target(); }
int annotated(void)
#if FLAG
__acquires(first)
#else
__acquires(second)
#endif
{ target(); }
extern API(result_t)
get(void) { return target(); }
static void printflike(3,4)
log(int x) { target(); }
static WRAP(int wrapped(int x)) { target(); }
int __NTH(nested(int x)) { target(); }
FT_LOCAL_DEF(FT_Error)
load(FT_Face face) { target(); }
#if FLAG
int
first(int x)
#else
int
second(int x)
#endif
{ target(); }
