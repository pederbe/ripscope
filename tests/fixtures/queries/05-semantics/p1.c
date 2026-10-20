struct page *pop(void) {
    struct page *p = first();
    return p;
}
void g(void)
{
#include "body.inc"
    helper();
}
#if A
void left(void) {
#else
void right(void) {
#endif
    after();
}
void outer(void) {
#define INNER(x) inner_call(x)
    INNER(1);
}
