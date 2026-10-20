int apply(int (*fn)(int value), int arg) { return fn(arg); }
int twice(int (*outer)(int (*inner)(int)), int y) { return outer(0) + y; }
void (*signal_like(int sig, void (*handler)(int)))(int) { return handler; }
int plain(int x) { return x; }
