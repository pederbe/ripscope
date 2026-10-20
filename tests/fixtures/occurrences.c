#define SAME(x) ((x) + global)
#ifdef UNKNOWN
int hidden;
#endif
typedef int Result;
int global;
struct Ops { int member; };
Result make(Result arg, struct Ops *ops) {
    Result local = SAME(arg);
    return local + ops->member;
}
Unknown build(Unknown value) {
    Unknown copy = value;
    return copy;
}
int one_line(void) { int leak; }
int leak;
