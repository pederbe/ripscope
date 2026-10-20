#include <stdio.h>
#include "local.h"
int counter;
#define BUMP(x) ((x) + counter)
static int helper(int value)
{
    counter = value;
    return BUMP(value);
}
int main(void)
{
    int local = helper(1);
    counter += local;
    if (counter == 2)
        printf("%d\n", counter);
    return 0;
}
