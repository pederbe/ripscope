typedef void (__cdecl *sig_handler_t)(int);
typedef unsigned int (WINAPI *IF_NAMETOINDEX_FN)(const char *);
typedef sds (aux_value_getter) (clusterNode* n, sds s);
typedef int (aux_value_present) (clusterNode* n);
typedef T (a)(int x), (b)(int y);
typedef enum cb_next (*cb_iter)(struct cb_node *, void *arg);
typedef enum rewrite_result (*rewrite_parent_fn_t)(struct rev_info *revs, struct commit **pp);
typedef union u *(*fp)(int x);
typedef T (*unknown)(int arg);
typedef void (API CALL *decorated)(const T *arg);
typedef int (*nested(int input))(int output);
typedef TYPEOF(left *right) (alias);
typedef TYPEOF(*ptr) (*pointer_alias);
typedef T (*opaque)(Other);
typedef T (*commented)(/* note */ int arg);
typedef T (*aggregate)(struct S { int member; } arg);
int after;
#if ENABLE_TAG
typedef struct ConditionalTag
#else
typedef struct
#endif
{ int member; } ConditionalAlias;
typedef TYPEOF(*ptr) (API decorated_alias);
typedef TYPEOF(*ptr) (API *decorated_pointer_alias);
