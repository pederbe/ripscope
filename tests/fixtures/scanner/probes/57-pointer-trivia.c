int *(*first)(int a) ATTR, (*second)(int b) OTHER;
int after;
struct ops {
    void (*reset) /* comment */
        (fsm *);
    int (*next) /* comment */ (fsm *, int *code) ATTR;
};
int (*x)/*gap*/(void); int count=(x=1);
T (*opaque) /* gap */ (Alias);
T (*aggregate) /* gap */ (struct S { int field; } arg);
int following;
API void * (*Alloc)(size_t bytes) ATTR;
TYPEOF(*ptr) /* gap */ (DECOR *global_alias);
