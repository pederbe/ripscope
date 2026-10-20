#if A
int f1(void) {
#elif B
int f2(void) {
#elif C
int f3(void) {
#elif D
int f4(void) {
#elif E
int f5(void) {
#elif F
int f6(void) {
#elif G
int f7(void) {
#elif H
int f8(void) {
#elif I
int f9(void) {
#elif J
int f10(void) {
#else
int f11(void) {
#endif
    target();
}
int top = target();
#define WRAP(v) target(v)
void body(void)
{
    first();
#define INNER(z) hidden(z)
    second();
}
void macros_after(void) { third(); }
