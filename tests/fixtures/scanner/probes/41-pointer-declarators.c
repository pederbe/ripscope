typedef unsigned long u64;
struct ops {
    u64 (*hash)(const void *key);
    int (*compare)(const void *a, const void *b);
    u64 (*table[4])(int);
    u64 (* const fixed)(void);
    void (^block)(int);
    u64 (*rows)[4];
    int (*krows)[4];
};
typedef u64 (*hash_fn)(const void *key);
static u64 (*global_hook)(int) = 0;
extern u64 (*external_hook)(int);
int (*first)(void), (*second)(void);
void install(u64 (*callback)(int value));
void run(void) { u64 (*local)(int) = 0; (*local)(1); }
u64 (*grows)[4];
static const u64 (*sg)[4] = 0;
int after;
