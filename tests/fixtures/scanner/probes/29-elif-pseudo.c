#if A
void one(void) {
#elif B
void two(void) {
#endif
#if C
void three(void) {
#endif
  call();
}
int after;
