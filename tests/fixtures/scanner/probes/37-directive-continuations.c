#if defined(A) || \
    (X < 5)
int f(void) { return 0; }
#endif
#if defined(B) && \
    defined(C)
int g;
#endif
int after;
