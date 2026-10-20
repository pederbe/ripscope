static int leaf(int value) { return value; }
int ordinary(void) { return leaf(1); }
int grouped(void) { return (leaf(1)); }
int split(void) {
    return (leaf
            (1));
}
int assigned(int value) {
    int result;
    if ((result = (leaf
                   (value)))) return result;
    return 0;
}
