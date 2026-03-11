/**
 * 
 * : UserService
 * : example.user.v1
 *
 * ⚠️  
 */

/**
 * GetUserRequest 
 */
export interface GetUserRequest {
  user_id: string;
}

/**
 * GetUserResponse 
 */
export interface GetUserResponse {
  user: User;
}

/**
 * CreateUserRequest 
 */
export interface CreateUserRequest {
  name: string;
  email: string;
  age?: number;
  tags: string[];
}

/**
 * CreateUserResponse 
 */
export interface CreateUserResponse {
  user: User;
}

/**
 * UpdateUserRequest 
 */
export interface UpdateUserRequest {
  user_id: string;
  name?: string;
  email?: string;
  age?: number;
}

/**
 * UpdateUserResponse 
 */
export interface UpdateUserResponse {
  user: User;
  updated: boolean;
}

/**
 * DeleteUserRequest 
 */
export interface DeleteUserRequest {
  user_id: string;
}

/**
 * DeleteUserResponse 
 */
export interface DeleteUserResponse {
  deleted: boolean;
}

/**
 * ListUsersRequest 
 */
export interface ListUsersRequest {
  page: number;
  page_size: number;
  filter?: string;
}

/**
 * WatchUsersRequest 
 */
export interface WatchUsersRequest {
  user_ids: string[];
}

/**
 * User 
 */
export interface User {
  id: string;
  name: string;
  email: string;
  age: number;
  tags: string[];
  created_at: number;
  updated_at: number;
  is_active: boolean;
}

/**
 * UserEvent 
 */
export interface UserEvent {
  event_type: string;
  user: User;
  timestamp: number;
}

