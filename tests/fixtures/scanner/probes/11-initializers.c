int table[4] = { 1, 2, compute(3), 4 };
struct config { int width; int height : 4; } defaults = { .width = 80, .height = 3 };
int matrix[ROWS][COLS];
static const char *names[] = { "a", "b", NULL };
int x = 1, y = 2, z;
