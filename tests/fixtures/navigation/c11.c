_Atomic(int) atomic_counter;
_Atomic int alternate_counter;
int following_counter;
_Thread_local int thread_counter;
_Alignas(16) int aligned_counter;
typedef _Atomic(int) atomic_alias;
struct anonymous_tag {
    union { int anonymous_member; double anonymous_real; };
};
_Static_assert(sizeof(int) >= 2, "int width");
int target(void) { return 1; }
int choose(void) {
    return _Generic(1, int: target(), default: 0);
}
int alignment_control(void) { return _Alignof(int); }
_Noreturn void never_returns(void) { for (;;) {} }
int after_c11(void) { return target(); }
