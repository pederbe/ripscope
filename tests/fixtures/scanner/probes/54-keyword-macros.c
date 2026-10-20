#define const
#define inline __inline__
#define struct(x) replacement(x)
#define int long
#define return target()
#define if(cond, ...) if ( __trace_if_var(!!(cond, ## __VA_ARGS__)) )
#undef const
int after;
