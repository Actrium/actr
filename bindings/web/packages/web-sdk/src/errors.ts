/**
 * Error handling for Actor-RTC Web
 */

/**
 * Error codes
 */
export enum ErrorCode {
  NetworkError = 'NETWORK_ERROR',
  TimeoutError = 'TIMEOUT_ERROR',
  ConnectionError = 'CONNECTION_ERROR',
  SerializationError = 'SERIALIZATION_ERROR',
  ServiceNotFound = 'SERVICE_NOT_FOUND',
  MethodNotFound = 'METHOD_NOT_FOUND',
  InternalError = 'INTERNAL_ERROR',
  ConfigError = 'CONFIG_ERROR',
}

/**
 * Actor error class
 */
export class ActorError extends Error {
  public readonly code: ErrorCode;
  public readonly details?: unknown;

  constructor(message: string, code: ErrorCode, details?: unknown) {
    super(message);
    this.name = 'ActorError';
    this.code = code;
    this.details = details;

    // Maintains proper stack trace in V8
    const ErrorWithCapture = Error as typeof Error & {
      captureStackTrace?: (target: object, constructor: Function) => void;
    };
    if (ErrorWithCapture.captureStackTrace) {
      ErrorWithCapture.captureStackTrace(this, ActorError);
    }
  }

  /**
   * Create a network error
   */
  static networkError(message: string, details?: unknown): ActorError {
    return new ActorError(message, ErrorCode.NetworkError, details);
  }

  /**
   * Create a timeout error
   */
  static timeoutError(message: string = 'Operation timed out'): ActorError {
    return new ActorError(message, ErrorCode.TimeoutError);
  }

  /**
   * Create a connection error
   */
  static connectionError(message: string, details?: unknown): ActorError {
    return new ActorError(message, ErrorCode.ConnectionError, details);
  }

  /**
   * Create a serialization error
   */
  static serializationError(message: string, details?: unknown): ActorError {
    return new ActorError(message, ErrorCode.SerializationError, details);
  }

  /**
   * Create a service not found error
   */
  static serviceNotFound(service: string): ActorError {
    return new ActorError(`Service not found: ${service}`, ErrorCode.ServiceNotFound, { service });
  }

  /**
   * Create a method not found error
   */
  static methodNotFound(service: string, method: string): ActorError {
    return new ActorError(`Method not found: ${service}.${method}`, ErrorCode.MethodNotFound, {
      service,
      method,
    });
  }

  /**
   * Create an internal error
   */
  static internalError(message: string, details?: unknown): ActorError {
    return new ActorError(message, ErrorCode.InternalError, details);
  }

  /**
   * Create a config error
   */
  static configError(message: string): ActorError {
    return new ActorError(message, ErrorCode.ConfigError);
  }

  /**
   * Convert from WASM error
   */
  static fromWasmError(error: unknown): ActorError {
    const message =
      error && typeof error === 'object' && 'message' in error
        ? String((error as { message: unknown }).message)
        : String(error);

    // Try to parse error code from message
    if (message.includes('Network')) {
      return ActorError.networkError(message);
    } else if (message.includes('Timeout')) {
      return ActorError.timeoutError(message);
    } else if (message.includes('Connection')) {
      return ActorError.connectionError(message);
    } else if (message.includes('Serialization')) {
      return ActorError.serializationError(message);
    }

    return ActorError.internalError(message);
  }
}
