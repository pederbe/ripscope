extern int counter;
void reset(void) { counter = 0; helper(2); }
int f(void) { return seen; }
int later = 2;
#define LAST counter \
