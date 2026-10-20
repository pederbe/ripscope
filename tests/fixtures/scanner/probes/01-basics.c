#include "local.h"
#include <system.h>
#define LIMIT 10
#define SQUARE(x) ((x) * (x))
int counter;
static int hidden = 3;
extern int elsewhere;
struct point { int x; int y; };
union value { int i; float f; };
enum color { RED, GREEN = 2, BLUE };
typedef unsigned long size_type;
typedef struct point point_t;
int add(int a, int b)
{
	int local = a + b;
	counter = counter + 1;
	return SQUARE(local) + helper(a);
}
static void reset(void) { counter = 0; hidden = 0; }
int main(void) { reset(); return add(1, 2); }
