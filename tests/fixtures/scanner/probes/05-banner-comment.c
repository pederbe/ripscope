#if A
int f(void) {
#endif
/*****/
#if B /* b */
int g(void) {
#endif
  a();
}
int after(void) { b(); }
