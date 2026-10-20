int same(int same) { return same + same; }
int calls(void) { f(); f(); g(f()); return 0; }
#define TWICE(v) v + v
