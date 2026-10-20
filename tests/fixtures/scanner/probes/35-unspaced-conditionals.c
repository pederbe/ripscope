int f(void) {
#if(A)
  {
#elif!defined(B)
  {
#endif
  g();
  }
}
int h;
