#define EMPTY
#define VALUE 42
#define CALLS(x) first(x); second(x)
#define MULTI(a, b) \
	do { \
		inner(a); \
		other(b); \
	} while (0)
#define BRACE { open(); }
#undef VALUE
int user(void) { return CALLS(1) + MULTI(2, 3); }
