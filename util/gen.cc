#include <iostream>
#include <cstdlib>
#include <ctime>

#include <kcpolydb.h>

int main(int argc, char **argv) {
    if (argc <= 1) {
        std::cout << "Usage: gen <dbfile.kct>" << std::endl;
        return 1;
    }

    kyotocabinet::TreeDB db;

    db.tune_options(kyotocabinet::TreeDB::TLINEAR);
    //db.tune_buckets(400000000L / 10 * 60);
    db.tune_buckets(10000);

    if (!db.open(argv[1], kyotocabinet::TreeDB::OWRITER | kyotocabinet::TreeDB::OCREATE)) {
        std::cout << "Could not open database." << std::endl;
        return 2;
    }

    std::cout << "Generating ..." << std::endl;

    char buf_key[16], buf_value[8];

    time_t t;
    std::srand(std::time(&t));

    for (long i = 0; i < 1000000; i++) {
        if (i % 50000 == 0) {
            std::cerr << ".";
        }

        for (int j = 0; j < 16; j++) {
            buf_key[j] = std::rand();
        }

        for (int j = 0; j < 8; j++) {
            buf_value[j] = std::rand();
        }

        if (!db.set(buf_key, 16, buf_value, 8)) {
            std::cout << "Error!" << std::endl;
            return 2;
        }
    }

    std::cerr << std::endl;
    return 0;
}
