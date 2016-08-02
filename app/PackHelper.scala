package lila.openingexplorer

import java.io.{ OutputStream, InputStream }
import chess.format.Uci
import chess.Pos
import chess.Role
import chess.{ Pawn, Knight, Bishop, Rook, Queen, King }

trait PackHelper {

  protected val MaxUint8: Int = 255
  protected val MaxUint16: Int = 65536
  protected val MaxUint24: Int = 16777215
  protected val MaxUint32: Long = 4294967295L
  protected val MaxUint48: Long = 281474976710655L


  protected def packUint8(v: Int): Array[Byte] =
    Array(v.toByte)

  protected def packUint8(v: Long): Array[Byte] =
    Array(v.toByte)

  protected def packUint16(v: Int): Array[Byte] =
    Array((0xff & (v >> 8)).toByte, (0xff & v).toByte)

  protected def packUint16(v: Long): Array[Byte] =
    packUint16(v.toInt)

  protected def packUint24(v: Int): Array[Byte] =
    packUint16((0xffff & (v >> 8))) ++ packUint8(0xff & v)

  protected def packUint24(v: Long): Array[Byte] =
    packUint24(v.toInt)

  protected def packUint32(v: Long): Array[Byte] =
    packUint16((0xffff & (v >> 16)).toInt) ++ packUint16((0xffff & v).toInt)

  protected def packUint48(v: Long): Array[Byte] =
    packUint32(0xffffffffL & (v >> 16)) ++ packUint16((0xffff & v).toInt)


  protected def unpackUint8(b: Array[Byte]): Int =
    b(0) & 0xff

  protected def unpackUint16(b: Array[Byte]): Int =
    unpackUint8(b) << 8 | unpackUint8(b.drop(1))

  protected def unpackUint24(b: Array[Byte]): Int =
    unpackUint16(b) << 8 | unpackUint8(b.drop(2))

  protected def unpackUint32(b: Array[Byte]): Long =
    unpackUint16(b).toLong << 16 | unpackUint16(b.drop(2)).toLong

  protected def unpackUint48(b: Array[Byte]): Long =
    unpackUint32(b) << 16 | unpackUint16(b.drop(4)).toLong


  protected def writeUint(stream: OutputStream, v: Long) = {
    var value = v
    while (value > 127) {
      stream.write(((value & 127) | 128).toInt)
      value >>= 7
    }
    stream.write((value & 127).toInt)
  }

  protected def readUint(stream: InputStream): Long = {
    var value: Long = 0
    var i: Int = 0
    var byte: Int = 0

    do {
      byte = stream.read()
      value |= (byte.toLong & 127) << (7 * i)
      i += 1
    } while ((byte & 128) != 0)

    value
  }


  protected def writeUint16(stream: OutputStream, v: Int) = {
    stream.write(0xff & (v >> 8))
    stream.write(0xff & v)
  }

  protected def readUint16(stream: InputStream): Int =
    stream.read() << 8 | stream.read()


  protected def writeUci(stream: OutputStream, move: Uci.Move): Unit =
    writeUint16(stream,
      Pos.all.indexOf(move.orig) |
      Pos.all.indexOf(move.dest) << 6 |
      move.promotion.fold(0)(r => (Role.allPromotable.indexOf(r)) + 1) << 12)

  protected def writeUci(stream: OutputStream, drop: Uci.Drop): Unit = {
    val dest = Pos.all.indexOf(drop.pos)
    writeUint16(stream, dest | dest << 6 | (Role.all.indexOf(drop.role) + 1) << 12)
  }

  protected def writeUci(stream: OutputStream, move: Either[Uci.Move, Uci.Drop]): Unit =
    move.fold(writeUci(stream, _), writeUci(stream, _))

  protected def readUci(stream: InputStream): Either[Uci.Move, Uci.Drop] = {
    val enc = readUint16(stream)
    val orig = Pos.all(enc & 63)
    val dest = Pos.all((enc >> 6) & 63)
    if (orig == dest) {
      Right(new Uci.Drop(Role.all((enc >> 12) - 1), dest))
    } else {
      val role = if ((enc >> 12) != 0) Some(Role.allPromotable((enc >> 12) - 1)) else None
      Left(new Uci.Move(orig, dest, role))
    }
  }
}
