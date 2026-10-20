#if CONFIG_A
int first(void) {
#else
int second(void) {
#endif
	call_in_body();
}
#ifdef CONFIG_B
static int guarded = 1;
#endif
#if A
int f(void) {
#endif
#if B
int g(void) {
#endif
	a();
}
int after(void) { b(); }
#if(C)
int unspaced(void) { c(); }
#endif
int tail(void) { d(); }
