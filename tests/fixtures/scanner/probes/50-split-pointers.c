typedef struct ops {
T (*read)
(struct buffer *p, int count);
int (*write)
(T *item, void (*notify)(void *arg));
T (API *decorated)
(T *first, U *second);
T (*empty)
();
int (*aggregate)
(struct param_tag { int real_member; } *p);
TYPEOF(x)
(alias);
TYPEOF(*ptr)
(other_alias);
T (*opaque)
(T *p);
T (*commented)
(int /* note */ q);
T (*rows)
[4];
} ops;
T (*first)
(int x), (*second)
(int y);
typedef int (*callback)
(int value);
extern T (*external)
(int ext_arg);
void owner(void) {
T (*local)
(int local_arg);
}
int
(wrapper)(int value)
{ return choose(left *right)
(value, value); }
void prototype(struct tag { int member; } value, T (*cb)
(int arg), int other);
int after;
