const signed char clean_table[2] = {1, 2};
void clean_writes(int *slots) {
    int counter = 0;
    counter++;
    --counter;
    slots[++counter] = 1;
    if (slots[0] == 1) counter += 1;
}
