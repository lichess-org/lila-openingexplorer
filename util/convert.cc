#include <iostream>
#include <string>

#include <kcpolydb.h>

int main(int argc, char **argv) {
  if (argc != 3) {
    std::cout << "Usage: convert <in> <out>" << std::endl;
    std::cout << "Copies records from in to out." << std::endl;
  }

  std::cout << "Waiting for read lock ..." << std::endl;
  kyotocabinet::PolyDB in;
  if (!in.open(argv[1], kyotocabinet::PolyDB::OREADER)) {
    std::cout << "Could not open source database." << std::endl;
    return 1;
  }

  std::cout << "Waiting for write lock ..." << std::endl;
  kyotocabinet::PolyDB out;
  if (!out.open(argv[2], kyotocabinet::PolyDB::OWRITER | kyotocabinet::PolyDB::OCREATE)) {
    std::cout << "Could not open target database." << std::endl;
    return 1;
  }

  std::cout << "Copying ..." << std::endl;
  long total = 0;
  std::string key, value;
  std::auto_ptr<kyotocabinet::PolyDB::Cursor> cur(in.cursor());
  cur->jump();
  while (cur->get(&key, &value, true)) {
    out.set(key, value);

    total++;
    if (total % 100000 == 0) {
      std::cerr << ".";
    }
  }

  std::cerr << std::endl;
  std::cout << "Done.";
  in.close();
  out.close();
  return 0;
}
