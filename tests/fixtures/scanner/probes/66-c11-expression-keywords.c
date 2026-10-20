#define CHOOSE(x) _Generic((x), int: macro_one(), default: macro_two())
#define ALIGN_VALUE _Alignof(int)
void expressions(int x) {
    before();
    _Generic(control(),
        int: chosen(),
        default: _Generic(x, int: alternate(), default: fallback()));
    after();
}
void alignment(void) {
    use(_Alignof(int));
    use(_Alignof(int[bound()]));
    _Alignof(int (*)(void));
    _Alignof(struct S);
}
void empty(void) {
    _Generic(0, int: 1, default: 2);
    _Alignof(int);
}
void conditional(void) {
#if SELECT
    _Generic(0, int: arm_one(), default: 0);
#else
    _Alignof(int[arm_two()]);
#endif
    neighbor();
}
void attributes(void) {
    __attribute__((aligned(_Alignof(int[attribute_bound()])))) int object;
    __attribute__((annotate(_Generic(0, int: attr_one(), default: attr_two())))) int other;
}
void spellings(void) {
    _Generic_helper(); _Alignof_helper();
    _Generic /* gap */ (0, int: gap_call(), default: 0);
    _Alignof
        (int[split_bound()]);
}
