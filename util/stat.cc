#include <iostream>
#include <string>

#include <kcpolydb.h>

int main(int argc, char **argv) {
    if (argc <= 1) {
      std::cout << "Usage: stat <dbfile>" << std::endl;
      std::cout << "Shows the distribution of the different pack formats." << std::endl;
      return 2;
    }

    kyotocabinet::PolyDB db;

    std::cout << "Waiting for read lock ..." << std::endl;

    if (!db.open(argv[1], kyotocabinet::PolyDB::OREADER)) {
      std::cout << "Could not open database." << std::endl;
      return 1;
    }

    std::auto_ptr<kyotocabinet::PolyDB::Cursor> cur(db.cursor());
    cur->jump();

    std::string key, value;

    long pack[] = {0, 0, 0, 0, 0, 0, 0};
    long total = 0;

    std::cout << "Scanning ..." << std::endl;

    while (cur->get(&key, &value, true)) {
        total++;
        if (value.size() == 8) {
            pack[0]++;
        } else {
            char c = value.at(0);

            if (0 <= c && c <= 6) {
                pack[c]++;
            } else {
                std::cout << "Error: Unknown pack format: " << c << std::endl;
                return 1;
            }
        }

        if (total % 50000 == 0) {
            std::cerr << ".";
        }
    }

    std::cerr << std::endl;

    for (int i = 0; i < 7; i++) {
        std::cout << "Pack format " << i << ": " << pack[i] << " nodes " << std::endl;
    }

    std::cout << "Unique positions: " << total << std::endl;

    return 0;
}
