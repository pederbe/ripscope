unsigned counter = 0;
#define STEP ++counter
#define WRAPPER(value) value
void step(void) {
    counter++;
    counter--;
    ++counter;
    --counter;
    ++(counter);
    ((counter))--;
    pointer++;
    *pointer++;
    ++*pointer;
    (*pointer)++;
    object.counter++;
    pointer->counter++;
    slots[index]++;
    ++object.counter;
    ++pointer->counter;
    ++slots[index];
    ((int)counter)++;
    slots[counter++] = value;
    sizeof(counter++);
    WRAPPER(--counter);
    for (; counter < limit; ++counter) helper();
    if (counter-- < limit) use(counter);
    sink(++counter);
    (void)counter++;
    (int)(counter++);
    read(counter); counter++;
    counter++; ++counter;
    left++; right--;
    other+++counter;
    other---counter;
    counter +/* gap */+other;
    counter -/* gap */-other;
    +\
+counter;
    counter+\
+;
    object-\
>counter++;
    counter == 1; counter != 1; counter <= 1; counter >= 1;
    counter; counter + 1; +counter; -counter; sizeof(counter);
    "counter++"; /* ++counter */
    counter += 1;
    counter = counter + 1;
}
void first(void) { read(counter); } counter++;
void second(void) { read(counter); #define INNER ++counter
}
int after;
