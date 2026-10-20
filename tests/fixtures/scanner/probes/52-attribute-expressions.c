__attribute__((annotate(payload()))) int prefix(void) {
    int __attribute__((annotate(helper()))) value;
    __declspec(align(bound())) int other;
}
int suffix(void) __attribute__((aligned(bound()))) { target(); }
typedef struct __attribute__((aligned(64))) state {
    unsigned long value;
} state;
struct __attribute__((packed)) { int value; } anonymous;
enum __attribute__((packed)) items { FIRST, SECOND };
union __declspec(align(16)) value { int i; double d; };
struct __attribute__((annotate("label"))) __attribute__((packed)) named { int value; };
struct __attribute__((packed)) forward *pointer;
#define DECL __attribute__((annotate(helper())))
int after;
typedef void (__attribute__((nocf_check)) *trigger_t)(void);
