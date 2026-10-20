#define WRITTEN_CALL(n) policy_leaf(n)
#define MAKE_NAME(n) int n
MAKE_NAME(expanded_only);
#if FIRST_ARM
int conditional_first;
#else
int conditional_second;
#endif
int policy_leaf(int n) { return n; }
int policy_caller(void) { return WRITTEN_CALL(1); }
int policy_after(void) { return policy_leaf(2); }
