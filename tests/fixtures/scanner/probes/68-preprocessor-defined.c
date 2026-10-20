#define MACRO_DEFINED(x) defined(x)
int defined(int value) { return value; }
int helper(void) { return 1; }
int call_defined(void) { return defined(2); }
int conditional(void) {
#if defined(OPTION) && CHECK(nested())
    return helper();
#elif !defined OTHER
    return alternate(inside());
#else
    return fallback();
#endif
}
int continued(void) {
#if OPTION && \
    defined (SECOND) && CHECK_SPLIT(inner_split())
    first_arm();
#elif !defined(THIRD)
    second_arm();
#endif
    return defined(3);
}
int unspaced(void) {
#if(defined(FOURTH))
    unspaced_arm();
#elif/* gap */ defined /* gap */ (FIFTH)
    comment_arm();
#endif
    return defined_neighbor();
}
int neighboring(void) {
    defined_neighbor();
    use(defined(4));
#pragma retain defined(5)
#ifdef defined
    true_branch();
#else
    false_branch();
#endif
    return 0;
}
